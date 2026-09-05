mod activity;
mod animation;
mod behavior;
mod browser_listener;
mod distraction;
mod intervention;
mod narrative;
mod world;

use activity::start_hyprland_activity_monitor;
use animation::MiloAnimator;
use behavior::BehaviorController;
use browser_listener::{BrowserActivityEvent, BrowserBridge, start_browser_activity_monitor};
use distraction::{DistractionController, DistractionEvent, SECOND_THRESHOLD_SECONDS};
use gtk::prelude::*;
use gtk4 as gtk;
use intervention::{
    Intervention, InterventionController, InterventionPresentation, InterventionResponse,
};
use milo::browser::{BrowserCommand, BrowserCommandResult};
use narrative::{DialogueLine, DialogueSequence, NarrativeController};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use world::{WorldEvent, WorldObject};

const APPLICATION_ID: &str = "com.milo.desktop";
const WINDOW_TITLE: &str = "Milo";
const MILO_DISPLAY_SIZE: i32 = 128;
const MILO_WINDOW_WIDTH: i32 = 184;

const ELI_PHOTO_APPEARANCE_DIALOGUE: &[&str] =
    &["I left it here.", "Thought you might want to see it."];
const ELI_PHOTO_INSPECTION_DIALOGUE: &[&str] = &[
    "That's me.",
    "The other name is Eli.",
    "...I haven't seen this in a long time.",
];

fn main() -> gtk::glib::ExitCode {
    let debug_intervention =
        std::env::args_os().any(|argument| argument == OsStr::new("--debug-intervention"));
    let application_arguments = std::env::args_os()
        .filter(|argument| argument != OsStr::new("--debug-intervention"))
        .collect::<Vec<_>>();
    let app = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .build();

    app.connect_activate(move |app| build_ui(app, debug_intervention));

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

    app.run_with_args_os(&application_arguments)
}

fn build_ui(app: &gtk::Application, debug_intervention: bool) {
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

            popover.milo-intervention > contents {
                border-radius: 12px;
                padding: 4px;
            }

            .milo-intervention-prompt {
                font-weight: bold;
            }

            popover.milo-narrative > contents {
                border-radius: 12px;
                padding: 10px;
            }

            button.eli-photo {
                background-color: #eee5d4;
                background-image: none;
                border: 1px solid #b9aa91;
                border-radius: 3px;
                box-shadow: 0 2px 5px alpha(#000000, 0.28);
                padding: 4px;
            }

            button.eli-photo:hover {
                background-color: #f7efdf;
            }

            .eli-photo-image {
                background-color: #706b64;
                border: 1px solid #554f48;
                border-radius: 1px;
            }

            .eli-photo-caption {
                background-color: #c7baa4;
                border-radius: 1px;
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

    let world_view = WorldView::new();
    let desktop = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    desktop.set_valign(gtk::Align::End);
    desktop.append(&picture);
    desktop.append(world_view.widget());

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(WINDOW_TITLE)
        .default_width(MILO_WINDOW_WIDTH)
        .default_height(MILO_DISPLAY_SIZE)
        .decorated(false)
        .resizable(false)
        .focusable(false)
        .focus_on_click(false)
        .child(&desktop)
        .build();
    window.add_css_class("milo-window");

    let animator = MiloAnimator::new(&picture).unwrap_or_else(|error| panic!("{error}"));
    let behavior = BehaviorController::new(animator);
    let browser_bridge = BrowserBridge::new();
    let intervention_view = InterventionView::new(&picture);
    let narrative_behavior = behavior.clone();
    let narrative_view = NarrativeView::new(&picture, move |active| {
        narrative_behavior.set_narrative_active(active);
    });
    let presentation = PresentationCoordinator::new(
        intervention_view.clone(),
        narrative_view,
        world_view.clone(),
        behavior.clone(),
    );
    let narrative_presentation = presentation.clone();
    let world_presentation = presentation.clone();
    let narrative = NarrativeController::load(
        move |dialogue| {
            narrative_presentation.enqueue_dialogue(dialogue);
        },
        move |event| {
            world_presentation.handle_world_event(event);
        },
    );
    world_view.set_object_visible(
        WorldObject::EliPhoto,
        narrative.is_world_object_visible(WorldObject::EliPhoto),
    );
    world_view.connect_interaction(&narrative);
    let intervention_presentation = presentation.clone();
    let response_browser_bridge = browser_bridge.clone();
    let response_narrative = narrative.clone();
    let intervention = Rc::new(InterventionController::new(
        move |presentation| {
            intervention_presentation.present_intervention(presentation);
        },
        move |response| {
            let Some(command) = browser_command_for_intervention_response(response) else {
                return;
            };

            response_narrative.break_accepted();
            eprintln!("[milo] browser command requested: {command:?}");
            match response_browser_bridge.send(command) {
                Ok(()) => eprintln!("[milo] browser command sent: {command:?}"),
                Err(error) => eprintln!("[milo] browser command unavailable: {error}"),
            }
        },
    ));
    intervention_view.connect_responses(&intervention);

    let distraction_behavior = behavior.clone();
    let distraction_intervention = Rc::clone(&intervention);
    let distraction_narrative = narrative.clone();
    let distraction = DistractionController::new(move |event| {
        distraction_behavior.handle_distraction_event(event);
        distraction_intervention.handle_distraction_event(event);
        if matches!(
            event,
            DistractionEvent::Threshold {
                seconds: SECOND_THRESHOLD_SECONDS,
                ..
            }
        ) {
            distraction_narrative.became_concerned();
        }
    });
    distraction.start_threshold_monitor();

    let idle_distraction = distraction.clone();
    let idle_intervention = Rc::clone(&intervention);
    let idle_narrative = narrative.clone();
    behavior.start_idle_monitor(move |system_idle| {
        idle_distraction.update_system_idle(system_idle);
        if system_idle {
            idle_intervention.system_idle();
            idle_narrative.system_idle();
        } else {
            idle_narrative.system_resumed();
        }
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
    start_browser_activity_monitor(browser_bridge, move |event| match event {
        BrowserActivityEvent::Activity(activity) => {
            eprintln!("[milo] browser activity: {activity:?}");
            distraction.update_browser_activity(activity);
        }
        BrowserActivityEvent::CommandResult { result, .. } => {
            log_browser_command_result(result);
        }
        BrowserActivityEvent::Unavailable => distraction.browser_unavailable(),
    });
    setup_native_drag(&window);
    setup_debug_state_switch(&window, behavior.clone());

    window.present();
    eprintln!("[milo] GTK: window presented");
    setup_narrative_startup(&picture, narrative);
    behavior.start_cozy_activity_scheduler();

    if debug_intervention {
        let debug_presentation = presentation.clone();
        gtk::glib::timeout_add_local_once(Duration::from_secs(1), move || {
            debug_presentation
                .present_intervention(InterventionPresentation::Show(Intervention::StillScrolling));
        });
    }
}

#[derive(Default)]
struct NarrativeStartupGuard {
    scheduled_or_triggered: Cell<bool>,
}

impl NarrativeStartupGuard {
    fn try_schedule(&self) -> bool {
        !self.scheduled_or_triggered.replace(true)
    }

    fn cancel_pending(&self) {
        self.scheduled_or_triggered.set(false);
    }
}

fn setup_narrative_startup(picture: &gtk::Picture, narrative: NarrativeController) {
    let guard = Rc::new(NarrativeStartupGuard::default());

    let mapped_narrative = narrative.clone();
    let mapped_guard = Rc::clone(&guard);
    picture.connect_map(move |picture| {
        schedule_narrative_startup(picture, mapped_narrative.clone(), Rc::clone(&mapped_guard));
    });

    // `present()` may map the widget synchronously, before the signal handler above exists.
    if picture.is_mapped() {
        schedule_narrative_startup(picture, narrative, guard);
    }
}

fn schedule_narrative_startup(
    picture: &gtk::Picture,
    narrative: NarrativeController,
    guard: Rc<NarrativeStartupGuard>,
) {
    if !guard.try_schedule() {
        return;
    }

    eprintln!("[milo] GTK: Milo mapped");
    let weak_picture = picture.downgrade();
    gtk::glib::idle_add_local_once(move || {
        let Some(picture) = weak_picture.upgrade() else {
            guard.cancel_pending();
            return;
        };
        if !picture.is_mapped() {
            guard.cancel_pending();
            return;
        }

        eprintln!("[milo] narrative startup ready");
        narrative.first_launch();
    });
}

#[derive(Clone)]
struct PresentationCoordinator {
    intervention: InterventionView,
    narrative: NarrativeView,
    world: WorldView,
    behavior: BehaviorController,
}

impl PresentationCoordinator {
    fn new(
        intervention: InterventionView,
        narrative: NarrativeView,
        world: WorldView,
        behavior: BehaviorController,
    ) -> Self {
        Self {
            intervention,
            narrative,
            world,
            behavior,
        }
    }

    fn present_intervention(&self, presentation: InterventionPresentation) {
        match presentation {
            InterventionPresentation::Show(_) => {
                self.behavior.set_intervention_visible(true);
                self.narrative.set_intervention_visible(true);
                self.intervention.present(presentation);
            }
            InterventionPresentation::Hide => {
                self.intervention.present(presentation);
                self.narrative.set_intervention_visible(false);
                self.behavior.set_intervention_visible(false);
            }
        }
    }

    fn enqueue_dialogue(&self, dialogue: DialogueSequence) {
        self.narrative.enqueue(dialogue);
    }

    fn handle_world_event(&self, event: WorldEvent) {
        if event == WorldEvent::EliPhotoAppeared {
            self.world.set_object_visible(WorldObject::EliPhoto, true);
        }
        if let Some(dialogue) = dialogue_for_world_event(event) {
            self.enqueue_dialogue(dialogue);
        }
    }
}

fn dialogue_for_world_event(event: WorldEvent) -> Option<DialogueSequence> {
    match event {
        WorldEvent::ObjectPending(_) => None,
        WorldEvent::EliPhotoAppeared => Some(DialogueSequence::new(ELI_PHOTO_APPEARANCE_DIALOGUE)),
        WorldEvent::ObjectInspected(WorldObject::EliPhoto) => {
            Some(DialogueSequence::new(ELI_PHOTO_INSPECTION_DIALOGUE))
        }
    }
}

#[derive(Clone)]
struct WorldView {
    eli_photo: gtk::Button,
}

impl WorldView {
    fn new() -> Self {
        let image = gtk::Box::new(gtk::Orientation::Vertical, 0);
        image.set_size_request(34, 34);
        image.add_css_class("eli-photo-image");

        let caption = gtk::Box::new(gtk::Orientation::Vertical, 0);
        caption.set_size_request(24, 3);
        caption.set_halign(gtk::Align::Center);
        caption.add_css_class("eli-photo-caption");

        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.append(&image);
        card.append(&caption);

        let eli_photo = gtk::Button::builder()
            .tooltip_text("An old photograph")
            .width_request(46)
            .height_request(56)
            .valign(gtk::Align::End)
            .margin_bottom(8)
            .child(&card)
            .visible(false)
            .build();
        eli_photo.add_css_class("eli-photo");
        eli_photo.set_focus_on_click(false);

        Self { eli_photo }
    }

    fn widget(&self) -> &gtk::Button {
        &self.eli_photo
    }

    fn set_object_visible(&self, object: WorldObject, visible: bool) {
        match object {
            WorldObject::EliPhoto => self.eli_photo.set_visible(visible),
        }
    }

    fn connect_interaction(&self, narrative: &NarrativeController) {
        let narrative = narrative.clone();
        self.eli_photo.connect_clicked(move |_| {
            narrative.inspect_world_object(WorldObject::EliPhoto);
        });
    }
}

#[derive(Default)]
struct NarrativeViewState {
    queued_lines: VecDeque<DialogueLine>,
    current_line: Option<DialogueLine>,
    timeout: Option<gtk::glib::SourceId>,
    intervention_visible: bool,
    active: bool,
}

type NarrativeActivityHandler = RefCell<Box<dyn FnMut(bool)>>;

struct NarrativeViewInner {
    popover: gtk::Popover,
    label: gtk::Label,
    state: RefCell<NarrativeViewState>,
    activity_handler: NarrativeActivityHandler,
}

#[derive(Clone)]
struct NarrativeView {
    inner: Rc<NarrativeViewInner>,
}

impl NarrativeView {
    fn new<F>(anchor: &gtk::Picture, activity_handler: F) -> Self
    where
        F: FnMut(bool) + 'static,
    {
        let label = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .wrap(true)
            .max_width_chars(32)
            .build();
        let popover = gtk::Popover::builder()
            .autohide(false)
            .has_arrow(true)
            .position(gtk::PositionType::Top)
            .child(&label)
            .build();
        popover.add_css_class("milo-narrative");
        popover.set_can_target(false);
        popover.set_parent(anchor);

        Self {
            inner: Rc::new(NarrativeViewInner {
                popover,
                label,
                state: RefCell::new(NarrativeViewState::default()),
                activity_handler: RefCell::new(Box::new(activity_handler)),
            }),
        }
    }

    fn enqueue(&self, dialogue: DialogueSequence) {
        let mut lines = dialogue.into_lines().peekable();
        if lines.peek().is_none() {
            return;
        }

        let became_active = {
            let mut state = self.inner.state.borrow_mut();
            state.queued_lines.extend(lines);
            if state.active {
                false
            } else {
                state.active = true;
                true
            }
        };
        if became_active {
            (self.inner.activity_handler.borrow_mut())(true);
        }
        self.show_next_line();
    }

    fn set_intervention_visible(&self, visible: bool) {
        let timeout = {
            let mut state = self.inner.state.borrow_mut();
            state.intervention_visible = visible;
            if visible { state.timeout.take() } else { None }
        };
        if let Some(timeout) = timeout {
            timeout.remove();
        }
        if visible {
            self.inner.popover.popdown();
        } else {
            self.show_next_line();
        }
    }

    fn show_next_line(&self) {
        let (line, is_new) = {
            let mut state = self.inner.state.borrow_mut();
            if state.intervention_visible || state.timeout.is_some() {
                return;
            }

            match state.current_line {
                Some(line) => (line, false),
                None => {
                    let Some(line) = state.queued_lines.pop_front() else {
                        let became_inactive = state.active;
                        state.active = false;
                        drop(state);
                        self.inner.popover.popdown();
                        if became_inactive {
                            (self.inner.activity_handler.borrow_mut())(false);
                        }
                        return;
                    };
                    state.current_line = Some(line);
                    (line, true)
                }
            }
        };

        self.inner.label.set_label(line.text);
        if is_new {
            eprintln!("[milo] narrative dialogue: {:?}", line.text);
        }
        self.inner.popover.popup();

        let weak_inner = Rc::downgrade(&self.inner);
        let timeout = gtk::glib::timeout_add_local_once(line.duration, move || {
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            {
                let mut state = inner.state.borrow_mut();
                state.timeout.take();
                state.current_line = None;
            }
            inner.popover.popdown();
            Self { inner }.show_next_line();
        });
        self.inner.state.borrow_mut().timeout = Some(timeout);
    }
}

fn log_browser_command_result(result: BrowserCommandResult) {
    eprintln!("[milo] browser command result: {result}");
}

fn browser_command_for_intervention_response(
    response: InterventionResponse,
) -> Option<BrowserCommand> {
    match response {
        InterventionResponse::TakeBreak => Some(BrowserCommand::CloseActiveDistractionTab),
        InterventionResponse::KeepScrolling => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialogue_text(dialogue: DialogueSequence) -> Vec<&'static str> {
        dialogue.into_lines().map(|line| line.text).collect()
    }

    #[test]
    fn only_take_break_maps_to_a_browser_command() {
        assert_eq!(
            browser_command_for_intervention_response(InterventionResponse::TakeBreak),
            Some(BrowserCommand::CloseActiveDistractionTab)
        );
        assert_eq!(
            browser_command_for_intervention_response(InterventionResponse::KeepScrolling),
            None
        );
    }

    #[test]
    fn narrative_startup_guard_allows_only_one_scheduled_trigger() {
        let guard = NarrativeStartupGuard::default();

        assert!(guard.try_schedule());
        assert!(!guard.try_schedule());

        guard.cancel_pending();
        assert!(guard.try_schedule());
        assert!(!guard.try_schedule());
    }

    #[test]
    fn world_events_map_to_the_expected_dialogue() {
        assert_eq!(
            dialogue_text(dialogue_for_world_event(WorldEvent::EliPhotoAppeared).unwrap()),
            ["I left it here.", "Thought you might want to see it."]
        );
        assert_eq!(
            dialogue_text(
                dialogue_for_world_event(WorldEvent::ObjectInspected(WorldObject::EliPhoto))
                    .unwrap()
            ),
            [
                "That's me.",
                "The other name is Eli.",
                "...I haven't seen this in a long time."
            ]
        );
        assert!(
            dialogue_for_world_event(WorldEvent::ObjectPending(WorldObject::EliPhoto)).is_none()
        );
    }
}

#[derive(Clone)]
struct InterventionView {
    popover: gtk::Popover,
    prompt: gtk::Label,
    take_break: gtk::Button,
    keep_scrolling: gtk::Button,
}

impl InterventionView {
    fn new(anchor: &gtk::Picture) -> Self {
        let prompt = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .css_classes(["milo-intervention-prompt"])
            .build();
        let take_break = gtk::Button::with_label(InterventionResponse::TakeBreak.label());
        take_break.set_focus_on_click(false);
        let keep_scrolling = gtk::Button::with_label(InterventionResponse::KeepScrolling.label());
        keep_scrolling.set_focus_on_click(false);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.append(&take_break);
        actions.append(&keep_scrolling);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.set_margin_top(10);
        content.set_margin_bottom(10);
        content.set_margin_start(10);
        content.set_margin_end(10);
        content.append(&prompt);
        content.append(&actions);

        let popover = gtk::Popover::builder()
            .autohide(false)
            .has_arrow(true)
            .position(gtk::PositionType::Top)
            .child(&content)
            .build();
        popover.add_css_class("milo-intervention");
        popover.set_parent(anchor);

        Self {
            popover,
            prompt,
            take_break,
            keep_scrolling,
        }
    }

    fn connect_responses(&self, controller: &Rc<InterventionController>) {
        let take_break_controller = Rc::downgrade(controller);
        self.take_break.connect_clicked(move |_| {
            if let Some(controller) = take_break_controller.upgrade() {
                controller.respond(InterventionResponse::TakeBreak);
            }
        });

        let keep_scrolling_controller = Rc::downgrade(controller);
        self.keep_scrolling.connect_clicked(move |_| {
            if let Some(controller) = keep_scrolling_controller.upgrade() {
                controller.respond(InterventionResponse::KeepScrolling);
            }
        });
    }

    fn present(&self, presentation: InterventionPresentation) {
        match presentation {
            InterventionPresentation::Show(intervention) => {
                self.prompt.set_label(intervention.prompt());
                eprintln!("[milo] GTK: calling intervention popover.popup()");
                self.popover.popup();
            }
            InterventionPresentation::Hide => self.popover.popdown(),
        }
    }
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
