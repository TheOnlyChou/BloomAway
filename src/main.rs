mod activity;
mod animation;
mod behavior;
mod browser_listener;
mod distraction;

use activity::start_hyprland_activity_monitor;
use animation::MiloAnimator;
use behavior::BehaviorController;
use browser_listener::{BrowserActivityEvent, start_browser_activity_monitor};
use distraction::DistractionController;
use gtk::prelude::*;
use gtk4 as gtk;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

const APPLICATION_ID: &str = "com.milo.desktop";
const WINDOW_TITLE: &str = "Milo";
const MILO_DISPLAY_SIZE: i32 = 128;

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

    let picture = gtk::Picture::builder()
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

    let animator = MiloAnimator::new(&picture).unwrap_or_else(|error| panic!("{error}"));
    let behavior = BehaviorController::new(animator);
    let distraction = DistractionController::new();
    distraction.start_threshold_monitor();

    let idle_distraction = distraction.clone();
    behavior.start_idle_monitor(move |system_idle| {
        idle_distraction.update_system_idle(system_idle);
    });
    let activity_behavior = behavior.clone();
    let desktop_distraction = distraction.clone();
    start_hyprland_activity_monitor(
        APPLICATION_ID,
        move |previous, current| {
            activity_behavior.handle_category_change(previous, current);
        },
        move |current| {
            desktop_distraction.update_desktop_activity(current);
        },
    );
    start_browser_activity_monitor(move |event| match event {
        BrowserActivityEvent::Activity(activity) => {
            eprintln!("[milo] browser activity: {activity:?}");
            distraction.update_browser_activity(activity);
        }
        BrowserActivityEvent::Unavailable => distraction.browser_unavailable(),
    });
    setup_native_drag(&window);
    setup_debug_state_switch(&window, behavior);

    window.present();
}

fn setup_debug_state_switch(window: &gtk::ApplicationWindow, behavior: BehaviorController) {
    let click = gtk::GestureClick::new();
    click.set_button(3);
    click.connect_pressed(move |_, _, _, _| {
        let next_state = behavior.cycle_debug_state();
        eprintln!("Milo state: {next_state:?}");
    });

    window.add_controller(click);
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
