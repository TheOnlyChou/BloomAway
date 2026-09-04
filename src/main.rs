use gtk::prelude::*;
use gtk4 as gtk;
use std::path::Path;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

const APPLICATION_ID: &str = "com.milo.desktop";
const WINDOW_TITLE: &str = "Milo";
const MILO_DISPLAY_SIZE: i32 = 128;

const IDLE_FRAME_PATHS: [&str; 4] = [
    "assets/milo/idle/idle_01.png",
    "assets/milo/idle/idle_02.png",
    "assets/milo/idle/idle_03.png",
    "assets/milo/idle/idle_04.png",
];

#[derive(Clone, Copy)]
struct AnimationStep {
    frame: usize,
    duration_ms: u64,
}

const IDLE_STEPS: [AnimationStep; 5] = [
    AnimationStep {
        frame: 0,
        duration_ms: 600,
    },
    AnimationStep {
        frame: 1,
        duration_ms: 350,
    },
    AnimationStep {
        frame: 2,
        duration_ms: 150,
    },
    AnimationStep {
        frame: 3,
        duration_ms: 350,
    },
    AnimationStep {
        frame: 0,
        duration_ms: 900,
    },
];

fn main() -> gtk::glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .build();

    app.connect_activate(build_ui);

    let should_quit = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&should_quit);
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::Relaxed))
        .expect("failed to install Ctrl+C handler");

    let timer_app = app.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
        if should_quit.load(Ordering::Relaxed) {
            timer_app.quit();
            gtk::glib::ControlFlow::Break
        } else {
            gtk::glib::ControlFlow::Continue
        }
    });

    app.run()
}

fn build_ui(app: &gtk::Application) {
    let css = gtk::CssProvider::new();
    css.load_from_data(
        r#"
            window.milo-window {
                background-color: transparent;
                background-image: none;
                border: none;
                box-shadow: none;
                margin: 0;
                padding: 0;
            }

            .milo-picture {
                background-color: transparent;
                margin: 0;
                padding: 0;
            }
        "#,
    );

    let display = gtk::gdk::Display::default().expect("Milo needs a graphical display");
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let idle_frames = load_idle_frames().unwrap_or_else(|error| panic!("{error}"));
    let picture = gtk::Picture::builder()
        .paintable(&idle_frames[0])
        .alternative_text("Milo")
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .width_request(MILO_DISPLAY_SIZE)
        .height_request(MILO_DISPLAY_SIZE)
        .css_classes(["milo-picture"])
        .build();

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(WINDOW_TITLE)
        .default_width(MILO_DISPLAY_SIZE)
        .default_height(MILO_DISPLAY_SIZE)
        .decorated(false)
        .resizable(false)
        .focusable(false)
        .focus_on_click(false)
        .child(&picture)
        .build();
    window.add_css_class("milo-window");

    setup_native_drag(&window);
    start_idle_animation(&picture, idle_frames);

    window.present();
}

fn load_idle_frames() -> Result<Vec<gtk::gdk::Texture>, String> {
    IDLE_FRAME_PATHS
        .iter()
        .map(|relative_path| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
            gtk::gdk::Texture::from_filename(&path)
                .map_err(|error| format!("failed to load idle frame {}: {error}", path.display()))
        })
        .collect()
}

fn start_idle_animation(picture: &gtk::Picture, frames: Vec<gtk::gdk::Texture>) {
    schedule_idle_step(picture.downgrade(), Rc::new(frames), 0);
}

fn schedule_idle_step(
    weak_picture: gtk::glib::WeakRef<gtk::Picture>,
    frames: Rc<Vec<gtk::gdk::Texture>>,
    step_index: usize,
) {
    let step = IDLE_STEPS[step_index];
    gtk::glib::timeout_add_local_once(Duration::from_millis(step.duration_ms), move || {
        let Some(picture) = weak_picture.upgrade() else {
            return;
        };

        let next_step_index = (step_index + 1) % IDLE_STEPS.len();
        let next_step = IDLE_STEPS[next_step_index];
        picture.set_paintable(Some(&frames[next_step.frame]));
        schedule_idle_step(picture.downgrade(), frames, next_step_index);
    });
}

fn setup_native_drag(window: &gtk::ApplicationWindow) {
    let click = gtk::GestureClick::new();
    click.set_button(1);

    let weak_window = window.downgrade();
    click.connect_pressed(move |gesture, _, _, _| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let Some(event) = gesture.current_event() else {
            return;
        };
        let Some(device) = event.device() else {
            return;
        };
        let Some((x, y)) = event.position() else {
            return;
        };
        let Some(surface) = window.surface() else {
            return;
        };
        let Ok(toplevel) = surface.downcast::<gtk::gdk::Toplevel>() else {
            return;
        };

        toplevel.begin_move(&device, gesture.current_button() as i32, x, y, event.time());
    });

    window.add_controller(click);
}
