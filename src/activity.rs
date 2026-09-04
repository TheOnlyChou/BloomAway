use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use gtk4::glib;
use std::env;
use std::fmt;
use std::io::{self, BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

const ACTIVE_WINDOW_PREFIX: &str = "activewindow>>";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveWindow {
    pub class: String,
    pub title: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityCategory {
    Browser,
    Development,
    Terminal,
    Gaming,
    Media,
    Other,
}

impl fmt::Display for ActivityCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

pub fn classify_window(window: &ActiveWindow) -> ActivityCategory {
    match window.class.trim().to_ascii_lowercase().as_str() {
        "firefox" | "google-chrome" | "chromium" => ActivityCategory::Browser,
        "code" | "code-oss" => ActivityCategory::Development,
        "kitty" | "alacritty" => ActivityCategory::Terminal,
        "steam" => ActivityCategory::Gaming,
        "spotify" => ActivityCategory::Media,
        _ => ActivityCategory::Other,
    }
}

pub fn start_hyprland_activity_monitor<F, G>(
    milo_class: &str,
    mut on_category_change: F,
    mut on_current_category: G,
) where
    F: FnMut(ActivityCategory, ActivityCategory) + 'static,
    G: FnMut(ActivityCategory) + 'static,
{
    let (sender, mut receiver) = unbounded();
    let mut context = ActivityContext::new(milo_class);

    glib::MainContext::default().spawn_local(async move {
        while let Some(window) = receiver.next().await {
            let previous_category = context.current.as_ref().map(classify_window);
            let transition = context.observe(window);
            let current_category = context.current.as_ref().map(classify_window);

            if current_category != previous_category
                && let Some(current) = current_category
            {
                on_current_category(current);
            }
            if let Some((previous, current)) = transition {
                on_category_change(previous, current);
            }
        }
    });

    if let Err(error) = thread::Builder::new()
        .name("milo-hyprland-activity".into())
        .spawn(move || {
            if let Err(error) = run_hyprland_listener(sender) {
                eprintln!("[milo] Hyprland activity tracking unavailable: {error}");
            }
        })
    {
        eprintln!(
            "[milo] Hyprland activity tracking unavailable: failed to start listener: {error}"
        );
    }
}

struct ActivityContext {
    milo_class: String,
    current: Option<ActiveWindow>,
}

impl ActivityContext {
    fn new(milo_class: &str) -> Self {
        Self {
            milo_class: milo_class.to_ascii_lowercase(),
            current: None,
        }
    }

    fn observe(&mut self, window: ActiveWindow) -> Option<(ActivityCategory, ActivityCategory)> {
        if window.class.trim().is_empty()
            || window.class.trim().eq_ignore_ascii_case(&self.milo_class)
            || self.current.as_ref() == Some(&window)
        {
            return None;
        }

        let previous_category = self.current.as_ref().map(classify_window);
        let current_category = classify_window(&window);
        self.current = Some(window);

        previous_category
            .filter(|previous| *previous != current_category)
            .map(|previous| (previous, current_category))
    }
}

fn run_hyprland_listener(sender: UnboundedSender<ActiveWindow>) -> Result<(), String> {
    let socket_path = hyprland_event_socket_path()?;
    let first_stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("could not open {}: {error}", socket_path.to_string_lossy()))?;

    if let Some(window) = query_initial_active_window()
        && sender.unbounded_send(window).is_err()
    {
        return Ok(());
    }

    let mut stream = first_stream;
    loop {
        match forward_events(stream, &sender) {
            Ok(ListenerExit::ReceiverClosed) => return Ok(()),
            Ok(ListenerExit::Disconnected) => {
                eprintln!("[milo] Hyprland activity socket disconnected; reconnecting");
            }
            Err(error) => {
                eprintln!("[milo] Hyprland activity socket error: {error}; reconnecting");
            }
        }

        loop {
            thread::sleep(RECONNECT_DELAY);
            match UnixStream::connect(&socket_path) {
                Ok(new_stream) => {
                    stream = new_stream;
                    eprintln!("[milo] Hyprland activity tracking reconnected");
                    break;
                }
                Err(_) if sender.is_closed() => return Ok(()),
                Err(_) => {}
            }
        }
    }
}

fn query_initial_active_window() -> Option<ActiveWindow> {
    let output = Command::new("hyprctl").arg("activewindow").output().ok()?;
    if !output.status.success() {
        return None;
    }

    parse_hyprctl_active_window(std::str::from_utf8(&output.stdout).ok()?)
}

fn parse_hyprctl_active_window(output: &str) -> Option<ActiveWindow> {
    let mut class = None;
    let mut title = None;

    for line in output.lines() {
        let field = line.trim_start();
        if let Some(value) = field.strip_prefix("class:") {
            class = Some(value.trim_start().to_owned());
        } else if let Some(value) = field.strip_prefix("title:") {
            title = Some(value.trim_start().to_owned());
        }
    }

    Some(ActiveWindow {
        class: class?,
        title: title?,
    })
}

fn hyprland_event_socket_path() -> Result<PathBuf, String> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "XDG_RUNTIME_DIR is missing".to_owned())?;
    let instance_signature = env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "HYPRLAND_INSTANCE_SIGNATURE is missing".to_owned())?;

    Ok(PathBuf::from(runtime_dir)
        .join("hypr")
        .join(instance_signature)
        .join(".socket2.sock"))
}

enum ListenerExit {
    Disconnected,
    ReceiverClosed,
}

fn forward_events(
    stream: UnixStream,
    sender: &UnboundedSender<ActiveWindow>,
) -> io::Result<ListenerExit> {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        let Some(window) = parse_active_window_event(&line) else {
            continue;
        };

        if sender.unbounded_send(window).is_err() {
            return Ok(ListenerExit::ReceiverClosed);
        }
    }

    Ok(ListenerExit::Disconnected)
}

fn parse_active_window_event(event: &str) -> Option<ActiveWindow> {
    let payload = event.strip_prefix(ACTIVE_WINDOW_PREFIX)?;
    let (class, title) = payload.split_once(',')?;

    Some(ActiveWindow {
        class: class.to_owned(),
        title: title.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_title_with_commas() {
        assert_eq!(
            parse_active_window_event("activewindow>>firefox,One, two, three"),
            Some(ActiveWindow {
                class: "firefox".to_owned(),
                title: "One, two, three".to_owned(),
            })
        );
    }

    #[test]
    fn ignores_unrelated_and_malformed_events() {
        assert_eq!(parse_active_window_event("workspace>>2"), None);
        assert_eq!(parse_active_window_event("activewindow>>firefox"), None);
    }

    #[test]
    fn parses_initial_hyprctl_window() {
        let output = "Window 123 -> Project:\n\tclass: kitty\n\ttitle: Project, notes\n";

        assert_eq!(
            parse_hyprctl_active_window(output),
            Some(ActiveWindow {
                class: "kitty".to_owned(),
                title: "Project, notes".to_owned(),
            })
        );
    }

    #[test]
    fn classifies_classes_case_insensitively() {
        let window = ActiveWindow {
            class: "FiReFoX".to_owned(),
            title: String::new(),
        };

        assert_eq!(classify_window(&window), ActivityCategory::Browser);
    }

    #[test]
    fn retains_last_meaningful_window_when_milo_is_focused() {
        let mut context = ActivityContext::new("com.milo.desktop");
        let kitty = ActiveWindow {
            class: "kitty".to_owned(),
            title: "project".to_owned(),
        };
        let milo = ActiveWindow {
            class: "COM.MILO.DESKTOP".to_owned(),
            title: "Milo".to_owned(),
        };

        assert_eq!(context.observe(kitty.clone()), None);
        assert_eq!(context.observe(milo), None);
        assert_eq!(context.current.as_ref(), Some(&kitty));
    }

    #[test]
    fn does_not_report_an_unchanged_window_twice() {
        let mut context = ActivityContext::new("com.milo.desktop");
        let window = ActiveWindow {
            class: "code".to_owned(),
            title: "Milo".to_owned(),
        };

        assert_eq!(context.observe(window.clone()), None);
        assert_eq!(context.observe(window), None);
    }

    #[test]
    fn title_only_changes_update_context_without_reporting_a_transition() {
        let mut context = ActivityContext::new("com.milo.desktop");
        let first = ActiveWindow {
            class: "kitty".to_owned(),
            title: "fish".to_owned(),
        };
        let second = ActiveWindow {
            class: "kitty".to_owned(),
            title: "~/Projects/BloomAway".to_owned(),
        };

        assert_eq!(context.observe(first), None);
        assert_eq!(context.observe(second.clone()), None);
        assert_eq!(context.current.as_ref(), Some(&second));
    }

    #[test]
    fn reports_only_category_transitions() {
        let mut context = ActivityContext::new("com.milo.desktop");
        let terminal = ActiveWindow {
            class: "kitty".to_owned(),
            title: "project".to_owned(),
        };
        let browser = ActiveWindow {
            class: "firefox".to_owned(),
            title: "Milo docs".to_owned(),
        };

        assert_eq!(context.observe(terminal), None);
        assert_eq!(
            context.observe(browser),
            Some((ActivityCategory::Terminal, ActivityCategory::Browser))
        );
    }

    #[test]
    fn different_apps_in_the_same_category_do_not_report_a_transition() {
        let mut context = ActivityContext::new("com.milo.desktop");
        let firefox = ActiveWindow {
            class: "firefox".to_owned(),
            title: "Tab A".to_owned(),
        };
        let chromium = ActiveWindow {
            class: "chromium".to_owned(),
            title: "Tab B".to_owned(),
        };

        assert_eq!(context.observe(firefox), None);
        assert_eq!(context.observe(chromium), None);
    }
}
