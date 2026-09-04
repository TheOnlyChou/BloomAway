use gtk::prelude::*;
use gtk4 as gtk;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

const INITIAL_X: i32 = 50;
const INITIAL_Y: i32 = 50;
const MIN_WIDTH: i32 = 140;
const MIN_HEIGHT: i32 = 80;

#[derive(Clone, Copy)]
struct Position {
    x: i32,
    y: i32,
}

fn main() -> gtk::glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("dev.milo.Companion")
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
            }

            .milo-placeholder {
                background-color: transparent;
                color: #ffffff;
                font-size: 24px;
                font-weight: bold;
                opacity: 1;
            }
        "#,
    );

    let display = gtk::gdk::Display::default().expect("Milo needs a graphical display");
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let label = gtk::Label::new(Some("Milo 🐈"));
    label.add_css_class("milo-placeholder");
    label.set_size_request(MIN_WIDTH, MIN_HEIGHT);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Milo")
        .decorated(false)
        .resizable(false)
        .focusable(false)
        .child(&label)
        .build();
    window.add_css_class("milo-window");

    setup_layer_shell(&window);
    setup_drag(&window);

    window.present();
}

fn setup_layer_shell(window: &gtk::ApplicationWindow) {
    window.init_layer_shell();
    window.set_namespace(Some("milo"));
    window.set_layer(Layer::Overlay);

    // A top-left anchor lets the margins act as x/y coordinates. Anchoring
    // opposite edges would stretch the surface across the monitor.
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, false);
    window.set_anchor(Edge::Bottom, false);
    set_position(
        window,
        Position {
            x: INITIAL_X,
            y: INITIAL_Y,
        },
    );
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);
}

fn setup_drag(window: &gtk::ApplicationWindow) {
    let position = Rc::new(Cell::new(Position {
        x: INITIAL_X,
        y: INITIAL_Y,
    }));
    let drag_start = Rc::new(Cell::new(position.get()));

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);

    let begin_position = Rc::clone(&position);
    let begin_start = Rc::clone(&drag_start);
    drag.connect_drag_begin(move |_, _, _| begin_start.set(begin_position.get()));

    let update_position = Rc::clone(&position);
    let update_start = Rc::clone(&drag_start);
    let weak_window = window.downgrade();
    drag.connect_drag_update(move |_, offset_x, offset_y| {
        let Some(window) = weak_window.upgrade() else {
            return;
        };

        let start = update_start.get();
        let candidate = Position {
            x: (f64::from(start.x) + offset_x).round() as i32,
            y: (f64::from(start.y) + offset_y).round() as i32,
        };
        let next = clamp_position(&window, candidate);

        set_position(&window, next);
        update_position.set(next);
    });

    window.add_controller(drag);
}

fn set_position(window: &gtk::ApplicationWindow, position: Position) {
    window.set_margin(Edge::Left, position.x);
    window.set_margin(Edge::Top, position.y);
}

fn clamp_position(window: &gtk::ApplicationWindow, position: Position) -> Position {
    let Some(limits) = position_limits(window) else {
        return Position {
            x: position.x.max(0),
            y: position.y.max(0),
        };
    };

    Position {
        x: position.x.clamp(0, limits.x),
        y: position.y.clamp(0, limits.y),
    }
}

fn position_limits(window: &gtk::ApplicationWindow) -> Option<Position> {
    if !window.is_mapped() {
        return None;
    }

    let width = window.width();
    let height = window.height();
    if width <= 0 || height <= 0 {
        return None;
    }

    let surface = window.surface()?;
    let monitor = surface.display().monitor_at_surface(&surface)?;
    let geometry = monitor.geometry();

    Some(Position {
        x: (geometry.width() - width).max(0),
        y: (geometry.height() - height).max(0),
    })
}
