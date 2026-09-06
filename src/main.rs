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
use gtk::gdk::prelude::GdkCairoContextExt;
use gtk::prelude::*;
use gtk4 as gtk;
use intervention::{
    Intervention, InterventionController, InterventionPresentation, InterventionResponse,
};
use milo::browser::{BrowserCommand, BrowserCommandResult};
use narrative::{DialogueLine, DialogueSequence, NarrativeController, NarrativeTrigger};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use world::{WorldEvent, WorldObject};

const APPLICATION_ID: &str = "com.milo.desktop";
const WINDOW_TITLE: &str = "Milo";
const MILO_ROOM_PATH: &str = "assets/world/room/milo_room.png";
const TRANSPARENT_WINDOW_WIDTH: i32 = 184;
const TRANSPARENT_WINDOW_HEIGHT: i32 = 128;
const TRANSPARENT_MILO_DISPLAY_SIZE: i32 = 128;
const ROOM_VIEWPORT_WIDTH: i32 = 400;
const ROOM_VIEWPORT_HEIGHT: i32 = 260;
const ROOM_MILO_DISPLAY_SIZE: i32 = 88;
const ROOM_FLOOR_BASELINE: i32 = 237;
const ROOM_MILO_X: i32 = 122;
const ROOM_MILO_Y: i32 = ROOM_FLOOR_BASELINE - ROOM_MILO_DISPLAY_SIZE;
const ELI_PHOTO_DISPLAY_WIDTH: i32 = 46;
const ELI_PHOTO_DISPLAY_HEIGHT: i32 = 58;
const ROOM_ELI_PHOTO_X: i32 = 239;
const ROOM_ELI_PHOTO_Y: i32 = ROOM_FLOOR_BASELINE - ELI_PHOTO_DISPLAY_HEIGHT;
const ROOM_CORNER_RADIUS: f64 = 22.0;
const ELI_PHOTO_THUMBNAIL_PATH: &str = "assets/world/eli_photo/eli_photo_thumbnail.png";
const ELI_PHOTO_FULL_PATH: &str = "assets/world/eli_photo/eli_photo_full.png";

const _: () = {
    assert!(ROOM_MILO_Y + ROOM_MILO_DISPLAY_SIZE == ROOM_FLOOR_BASELINE);
    assert!(ROOM_ELI_PHOTO_Y + ELI_PHOTO_DISPLAY_HEIGHT == ROOM_FLOOR_BASELINE);
    assert!(ROOM_MILO_X >= 0);
    assert!(ROOM_MILO_X + ROOM_MILO_DISPLAY_SIZE <= ROOM_VIEWPORT_WIDTH);
    assert!(ROOM_ELI_PHOTO_X >= 0);
    assert!(ROOM_ELI_PHOTO_X + ELI_PHOTO_DISPLAY_WIDTH <= ROOM_VIEWPORT_WIDTH);
    assert!(ROOM_FLOOR_BASELINE <= ROOM_VIEWPORT_HEIGHT);
};

const ELI_PHOTO_APPEARANCE_DIALOGUE: &[&str] =
    &["I left it here.", "Thought you might want to see it."];
const ELI_PHOTO_INSPECTION_DIALOGUE: &[&str] = &[
    "That's me.",
    "And that's Eli.",
    "We used to play with that all the time.",
    "...I haven't seen him in a long time.",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NarrativeSequenceKind {
    FirstLaunch,
    BecameConcerned,
    BreakAccepted,
    ReturnedAfterBreak,
    EliPhotoAppeared,
    EliPhotoInspection,
}

impl From<NarrativeTrigger> for NarrativeSequenceKind {
    fn from(trigger: NarrativeTrigger) -> Self {
        match trigger {
            NarrativeTrigger::FirstLaunch => Self::FirstLaunch,
            NarrativeTrigger::BecameConcerned => Self::BecameConcerned,
            NarrativeTrigger::BreakAccepted => Self::BreakAccepted,
            NarrativeTrigger::ReturnedAfterBreak => Self::ReturnedAfterBreak,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NarrativePresentationTarget {
    MiloBubble,
    PhotoInspection,
}

struct NarrativePresentation {
    kind: NarrativeSequenceKind,
    target: NarrativePresentationTarget,
    dialogue: DialogueSequence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InterventionResponsePlan {
    record_break_accepted: bool,
    browser_command: Option<BrowserCommand>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PresentationMode {
    #[default]
    TransparentCompanion,
    Room,
}

impl PresentationMode {
    fn from_arguments(arguments: &[OsString]) -> Self {
        arguments.iter().fold(Self::default(), |mode, argument| {
            if argument == OsStr::new("--room") {
                Self::Room
            } else if argument == OsStr::new("--transparent") {
                Self::TransparentCompanion
            } else {
                mode
            }
        })
    }

    fn is_mode_argument(argument: &OsStr) -> bool {
        argument == OsStr::new("--room") || argument == OsStr::new("--transparent")
    }

    fn milo_display_size(self) -> i32 {
        match self {
            Self::TransparentCompanion => TRANSPARENT_MILO_DISPLAY_SIZE,
            Self::Room => ROOM_MILO_DISPLAY_SIZE,
        }
    }

    fn window_size(self) -> (i32, i32) {
        match self {
            Self::TransparentCompanion => (TRANSPARENT_WINDOW_WIDTH, TRANSPARENT_WINDOW_HEIGHT),
            Self::Room => (ROOM_VIEWPORT_WIDTH, ROOM_VIEWPORT_HEIGHT),
        }
    }
}

fn main() -> gtk::glib::ExitCode {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    let debug_intervention = arguments
        .iter()
        .any(|argument| argument == OsStr::new("--debug-intervention"));
    let presentation_mode = PresentationMode::from_arguments(&arguments);
    let application_arguments = arguments
        .into_iter()
        .filter(|argument| {
            argument != OsStr::new("--debug-intervention")
                && !PresentationMode::is_mode_argument(argument)
        })
        .collect::<Vec<_>>();
    let app = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .build();

    app.connect_activate(move |app| build_ui(app, debug_intervention, presentation_mode));

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

fn build_ui(app: &gtk::Application, debug_intervention: bool, presentation_mode: PresentationMode) {
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
                background-color: #2a211d;
                color: #fff3df;
                border: 1px solid #80634f;
                border-radius: 12px;
                padding: 4px;
            }

            .milo-intervention-prompt {
                font-weight: bold;
            }

            popover.milo-narrative > contents {
                background-color: #2a211d;
                color: #fff3df;
                border: 1px solid #80634f;
                border-radius: 12px;
                padding: 10px;
            }

            button.eli-photo {
                background-color: transparent;
                background-image: none;
                border: none;
                box-shadow: none;
                padding: 0;
            }

            button.eli-photo:hover {
                background-color: alpha(#fff8e9, 0.14);
            }

            popover.eli-photo-inspection > contents {
                background-color: #241d19;
                border: 1px solid #6f5b4d;
                border-radius: 8px;
                padding: 8px;
            }

            .eli-photo-inspection-caption {
                color: #f6ead7;
                font-weight: bold;
                padding: 8px 6px 4px 6px;
            }
        "#,
    );

    let display = gtk::gdk::Display::default().expect("Milo needs a graphical display");
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let milo_display_size = presentation_mode.milo_display_size();
    let picture = gtk::Picture::builder()
        .alternative_text("Milo")
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .width_request(milo_display_size)
        .height_request(milo_display_size)
        .css_classes(["milo-picture"])
        .build();

    let world_view = WorldView::new().unwrap_or_else(|error| panic!("{error}"));
    let window_content = build_window_content(presentation_mode, &picture, &world_view)
        .unwrap_or_else(|error| panic!("{error}"));
    let (window_width, window_height) = presentation_mode.window_size();

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(WINDOW_TITLE)
        .default_width(window_width)
        .default_height(window_height)
        .decorated(false)
        .resizable(false)
        .focusable(false)
        .focus_on_click(false)
        .child(&window_content)
        .build();
    window.add_css_class("milo-window");

    let animator = MiloAnimator::new(&picture).unwrap_or_else(|error| panic!("{error}"));
    let behavior = BehaviorController::new(animator);
    let browser_bridge = BrowserBridge::new();
    let intervention_view = InterventionView::new(&picture);
    let narrative_behavior = behavior.clone();
    let narrative_view = NarrativeView::new(&picture, world_view.clone(), move |active| {
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
        move |trigger, dialogue| {
            narrative_presentation.enqueue_dialogue(NarrativePresentation {
                kind: trigger.into(),
                target: NarrativePresentationTarget::MiloBubble,
                dialogue,
            });
        },
        move |event| {
            world_presentation.handle_world_event(event);
        },
    );
    world_view.set_object_visible(
        WorldObject::EliPhoto,
        narrative.is_world_object_visible(WorldObject::EliPhoto),
    );
    world_view.connect_interaction(&narrative, &presentation);
    let intervention_presentation = presentation.clone();
    let response_browser_bridge = browser_bridge.clone();
    let response_narrative = narrative.clone();
    let intervention = Rc::new(InterventionController::new(
        move |presentation| {
            intervention_presentation.present_intervention(presentation);
        },
        move |response| {
            let plan = plan_intervention_response(response);
            if plan.record_break_accepted {
                response_narrative.break_accepted();
            }
            let Some(command) = plan.browser_command else {
                return;
            };

            let browser_bridge = response_browser_bridge.clone();
            gtk::glib::idle_add_local_once(move || {
                eprintln!("[milo] browser command requested: {command:?}");
                match browser_bridge.send(command) {
                    Ok(()) => eprintln!("[milo] browser command sent: {command:?}"),
                    Err(error) => eprintln!("[milo] browser command unavailable: {error}"),
                }
            });
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

fn build_window_content(
    presentation_mode: PresentationMode,
    picture: &gtk::Picture,
    world_view: &WorldView,
) -> Result<gtk::Widget, String> {
    match presentation_mode {
        PresentationMode::TransparentCompanion => {
            world_view.widget().set_valign(gtk::Align::End);
            world_view.widget().set_margin_bottom(8);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            content.set_valign(gtk::Align::End);
            content.append(picture);
            content.append(world_view.widget());
            Ok(content.upcast())
        }
        PresentationMode::Room => {
            let room_background = build_room_background()?;
            eprintln!("[milo] world background loaded: MiloRoom");
            let world_content = gtk::Fixed::new();
            world_content.put(picture, f64::from(ROOM_MILO_X), f64::from(ROOM_MILO_Y));
            world_content.put(
                world_view.widget(),
                f64::from(ROOM_ELI_PHOTO_X),
                f64::from(ROOM_ELI_PHOTO_Y),
            );

            let room = gtk::Overlay::new();
            room.set_child(Some(&room_background));
            room.add_overlay(&world_content);
            room.set_measure_overlay(&world_content, false);
            Ok(room.upcast())
        }
    }
}

fn build_room_background() -> Result<gtk::DrawingArea, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(MILO_ROOM_PATH);
    let pixbuf = gtk::gdk::gdk_pixbuf::Pixbuf::from_file(&path)
        .map_err(|error| format!("failed to load room artwork {}: {error}", path.display()))?;
    let background = gtk::DrawingArea::builder()
        .content_width(ROOM_VIEWPORT_WIDTH)
        .content_height(ROOM_VIEWPORT_HEIGHT)
        .build();
    background.set_can_target(false);
    background.set_draw_func(move |_, context, width, height| {
        let width = f64::from(width);
        let height = f64::from(height);
        add_rounded_rectangle(context, 0.0, 0.0, width, height, ROOM_CORNER_RADIUS);
        context.save().expect("failed to save room drawing context");
        context.clip();

        let image_width = f64::from(pixbuf.width());
        let image_height = f64::from(pixbuf.height());
        let scale = (width / image_width).max(height / image_height);
        let x = (width - image_width * scale) / 2.0;
        let y = (height - image_height * scale) / 2.0;
        context.translate(x, y);
        context.scale(scale, scale);
        context.set_source_pixbuf(&pixbuf, 0.0, 0.0);
        context.source().set_filter(gtk::cairo::Filter::Best);
        context.paint().expect("failed to paint room background");
        context
            .restore()
            .expect("failed to restore room drawing context");

        let border_inset = 0.75;
        add_rounded_rectangle(
            context,
            border_inset,
            border_inset,
            width - border_inset * 2.0,
            height - border_inset * 2.0,
            ROOM_CORNER_RADIUS - border_inset,
        );
        context.set_source_rgba(0.45, 0.29, 0.20, 0.75);
        context.set_line_width(border_inset * 2.0);
        context.stroke().expect("failed to paint room border");
    });

    Ok(background)
}

fn add_rounded_rectangle(
    context: &gtk::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    use std::f64::consts::{FRAC_PI_2, PI};

    context.new_sub_path();
    context.arc(x + width - radius, y + radius, radius, -FRAC_PI_2, 0.0);
    context.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        FRAC_PI_2,
    );
    context.arc(x + radius, y + height - radius, radius, FRAC_PI_2, PI);
    context.arc(x + radius, y + radius, radius, PI, PI + FRAC_PI_2);
    context.close_path();
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
        let coordinator = Self {
            intervention,
            narrative,
            world,
            behavior,
        };
        coordinator.connect_intervention_closed();
        coordinator
    }

    fn present_intervention(&self, presentation: InterventionPresentation) {
        match presentation {
            InterventionPresentation::Show(_) => {
                self.world.close_inspection();
                self.behavior.set_intervention_visible(true);
                self.narrative.set_intervention_visible(true);
                self.intervention.present(presentation);
            }
            InterventionPresentation::Hide => {
                let was_visible = self.intervention.is_visible();
                self.intervention.present(presentation);
                if !was_visible {
                    self.resume_after_intervention();
                }
            }
        }
    }

    fn connect_intervention_closed(&self) {
        let narrative = self.narrative.clone();
        let behavior = self.behavior.clone();
        self.intervention.connect_closed(move || {
            narrative.set_intervention_visible(false);
            behavior.set_intervention_visible(false);
        });
    }

    fn resume_after_intervention(&self) {
        self.narrative.set_intervention_visible(false);
        self.behavior.set_intervention_visible(false);
    }

    fn enqueue_dialogue(&self, presentation: NarrativePresentation) {
        self.narrative.enqueue(presentation);
    }

    fn handle_world_event(&self, event: WorldEvent) {
        if event == WorldEvent::EliPhotoAppeared {
            self.world.set_object_visible(WorldObject::EliPhoto, true);
        }
        if let Some(presentation) = dialogue_for_world_event(event) {
            self.enqueue_dialogue(presentation);
        }
    }
}

fn dialogue_for_world_event(event: WorldEvent) -> Option<NarrativePresentation> {
    match event {
        WorldEvent::ObjectPending(_) => None,
        WorldEvent::EliPhotoAppeared => Some(NarrativePresentation {
            kind: NarrativeSequenceKind::EliPhotoAppeared,
            target: NarrativePresentationTarget::MiloBubble,
            dialogue: DialogueSequence::new(ELI_PHOTO_APPEARANCE_DIALOGUE),
        }),
        WorldEvent::ObjectInspected(WorldObject::EliPhoto) => Some(NarrativePresentation {
            kind: NarrativeSequenceKind::EliPhotoInspection,
            target: NarrativePresentationTarget::PhotoInspection,
            dialogue: DialogueSequence::new(ELI_PHOTO_INSPECTION_DIALOGUE),
        }),
    }
}

#[derive(Clone)]
struct WorldView {
    eli_photo: gtk::Button,
    inspection: gtk::Popover,
    inspection_caption: gtk::Label,
}

impl WorldView {
    fn new() -> Result<Self, String> {
        let thumbnail_texture = load_world_texture(
            ELI_PHOTO_THUMBNAIL_PATH,
            ELI_PHOTO_DISPLAY_WIDTH,
            ELI_PHOTO_DISPLAY_HEIGHT,
        )?;
        let thumbnail = gtk::Picture::builder()
            .alternative_text("An old photograph of Milo and Eli")
            .paintable(&thumbnail_texture)
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .width_request(ELI_PHOTO_DISPLAY_WIDTH)
            .height_request(ELI_PHOTO_DISPLAY_HEIGHT)
            .build();

        let eli_photo = gtk::Button::builder()
            .tooltip_text("An old photograph")
            .width_request(ELI_PHOTO_DISPLAY_WIDTH)
            .height_request(ELI_PHOTO_DISPLAY_HEIGHT)
            .child(&thumbnail)
            .visible(false)
            .build();
        eli_photo.add_css_class("eli-photo");
        eli_photo.set_focus_on_click(false);

        let full_texture = load_world_texture(ELI_PHOTO_FULL_PATH, 320, 400)?;
        let full_photo = gtk::Picture::builder()
            .alternative_text("Milo and Eli playing together with a yarn ball")
            .paintable(&full_texture)
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .width_request(320)
            .height_request(400)
            .build();
        let inspection_caption = gtk::Label::builder()
            .halign(gtk::Align::Center)
            .wrap(true)
            .max_width_chars(40)
            .visible(false)
            .css_classes(["eli-photo-inspection-caption"])
            .build();
        let inspection_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        inspection_content.append(&full_photo);
        inspection_content.append(&inspection_caption);
        let inspection = gtk::Popover::builder()
            .autohide(true)
            .has_arrow(true)
            .position(gtk::PositionType::Top)
            .child(&inspection_content)
            .build();
        inspection.add_css_class("eli-photo-inspection");
        inspection.set_parent(&eli_photo);

        Ok(Self {
            eli_photo,
            inspection,
            inspection_caption,
        })
    }

    fn widget(&self) -> &gtk::Button {
        &self.eli_photo
    }

    fn set_object_visible(&self, object: WorldObject, visible: bool) {
        match object {
            WorldObject::EliPhoto => self.eli_photo.set_visible(visible),
        }
    }

    fn close_inspection(&self) {
        self.inspection_caption.set_visible(false);
        self.inspection.popdown();
    }

    fn show_inspection_line(&self, text: &str) {
        self.inspection_caption.set_label(text);
        self.inspection_caption.set_visible(true);
        self.inspection.popup();
    }

    fn hide_inspection_line(&self) {
        self.inspection_caption.set_visible(false);
    }

    fn connect_interaction(
        &self,
        narrative: &NarrativeController,
        presentation: &PresentationCoordinator,
    ) {
        let narrative = narrative.clone();
        let intervention = presentation.intervention.clone();
        let narrative_view = presentation.narrative.clone();
        let weak_inspection = self.inspection.downgrade();
        self.eli_photo.connect_clicked(move |_| {
            if intervention.is_visible() || narrative_view.is_active() {
                return;
            }
            let Some(inspection) = weak_inspection.upgrade() else {
                return;
            };

            inspection.popup();
            eprintln!("[milo] world object opened: EliPhoto");
            narrative.inspect_world_object(WorldObject::EliPhoto);
        });
    }
}

fn load_world_texture(
    relative_path: &str,
    width: i32,
    height: i32,
) -> Result<gtk::gdk::Texture, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let pixbuf = gtk::gdk::gdk_pixbuf::Pixbuf::from_file_at_scale(&path, width, height, true)
        .map_err(|error| format!("failed to load world artwork {}: {error}", path.display()))?;
    Ok(gtk::gdk::Texture::for_pixbuf(&pixbuf))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PresentedDialogueLine {
    target: NarrativePresentationTarget,
    line: DialogueLine,
}

#[derive(Debug, Eq, PartialEq)]
enum NarrativePump {
    SuppressedByIntervention,
    Present {
        line: PresentedDialogueLine,
        is_new: bool,
    },
    Finished {
        was_active: bool,
    },
}

#[derive(Default)]
struct NarrativePresentationState {
    queued_lines: VecDeque<PresentedDialogueLine>,
    current_line: Option<PresentedDialogueLine>,
    intervention_visible: bool,
    active: bool,
}

impl NarrativePresentationState {
    fn enqueue(&mut self, presentation: NarrativePresentation) -> bool {
        let NarrativePresentation {
            target, dialogue, ..
        } = presentation;
        self.queued_lines.extend(
            dialogue
                .into_lines()
                .map(|line| PresentedDialogueLine { target, line }),
        );
        if self.active {
            false
        } else {
            self.active = true;
            true
        }
    }

    fn set_intervention_visible(&mut self, visible: bool) -> bool {
        let changed = self.intervention_visible != visible;
        self.intervention_visible = visible;
        changed
    }

    fn pump(&mut self) -> NarrativePump {
        if self.intervention_visible {
            return NarrativePump::SuppressedByIntervention;
        }

        if let Some(line) = self.current_line {
            return NarrativePump::Present {
                line,
                is_new: false,
            };
        }

        if let Some(line) = self.queued_lines.pop_front() {
            self.current_line = Some(line);
            return NarrativePump::Present { line, is_new: true };
        }

        let was_active = self.active;
        self.active = false;
        NarrativePump::Finished { was_active }
    }

    fn complete_current_line(&mut self) -> Option<PresentedDialogueLine> {
        self.current_line.take()
    }
}

type NarrativeActivityHandler = RefCell<Box<dyn FnMut(bool)>>;

struct NarrativeViewInner {
    milo_popover: gtk::Popover,
    milo_label: gtk::Label,
    world: WorldView,
    state: RefCell<NarrativePresentationState>,
    timeout: RefCell<Option<gtk::glib::SourceId>>,
    activity_handler: NarrativeActivityHandler,
}

#[derive(Clone)]
struct NarrativeView {
    inner: Rc<NarrativeViewInner>,
}

impl NarrativeView {
    fn new<F>(anchor: &gtk::Picture, world: WorldView, activity_handler: F) -> Self
    where
        F: FnMut(bool) + 'static,
    {
        let milo_label = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .wrap(true)
            .max_width_chars(32)
            .build();
        let milo_popover = gtk::Popover::builder()
            .autohide(false)
            .has_arrow(true)
            .position(gtk::PositionType::Top)
            .child(&milo_label)
            .build();
        milo_popover.add_css_class("milo-narrative");
        milo_popover.set_can_target(false);
        milo_popover.set_parent(anchor);

        Self {
            inner: Rc::new(NarrativeViewInner {
                milo_popover,
                milo_label,
                world,
                state: RefCell::new(NarrativePresentationState::default()),
                timeout: RefCell::new(None),
                activity_handler: RefCell::new(Box::new(activity_handler)),
            }),
        }
    }

    fn enqueue(&self, presentation: NarrativePresentation) {
        if presentation.dialogue.clone().into_lines().next().is_none() {
            return;
        }
        eprintln!("[milo] narrative sequence queued: {:?}", presentation.kind);

        let (became_active, suppressed) = {
            let mut state = self.inner.state.borrow_mut();
            let became_active = state.enqueue(presentation);
            (became_active, state.intervention_visible)
        };
        if became_active {
            (self.inner.activity_handler.borrow_mut())(true);
        }
        if suppressed {
            eprintln!("[milo] narrative paused: intervention");
            eprintln!("[milo] narrative suppression: intervention");
        }
        self.show_next_line();
    }

    fn is_active(&self) -> bool {
        self.inner.state.borrow().active
    }

    fn set_intervention_visible(&self, visible: bool) {
        let (changed, active, current_target) = {
            let mut state = self.inner.state.borrow_mut();
            let changed = state.set_intervention_visible(visible);
            (
                changed,
                state.active,
                state.current_line.map(|line| line.target),
            )
        };
        if !changed {
            return;
        }

        if visible {
            if let Some(timeout) = self.inner.timeout.borrow_mut().take() {
                timeout.remove();
            }
            if active {
                eprintln!("[milo] narrative paused: intervention");
                eprintln!("[milo] narrative suppression: intervention");
            }
            self.hide_target_for_intervention(current_target);
        } else {
            if active {
                eprintln!("[milo] narrative resumed");
            }
            self.show_next_line();
        }
    }

    fn show_next_line(&self) {
        if self.inner.timeout.borrow().is_some() {
            return;
        }

        let pump = self.inner.state.borrow_mut().pump();
        let (presented, is_new) = match pump {
            NarrativePump::SuppressedByIntervention => return,
            NarrativePump::Finished { was_active } => {
                self.inner.milo_popover.popdown();
                self.inner.world.hide_inspection_line();
                if was_active {
                    (self.inner.activity_handler.borrow_mut())(false);
                }
                return;
            }
            NarrativePump::Present { line, is_new } => (line, is_new),
        };

        if is_new {
            eprintln!(
                "[milo] narrative presentation target: {:?}",
                presented.target
            );
            eprintln!("[milo] narrative dialogue: {:?}", presented.line.text);
        }
        self.show_target(presented);

        let weak_inner = Rc::downgrade(&self.inner);
        let timeout = gtk::glib::timeout_add_local_once(presented.line.duration, move || {
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            inner.timeout.borrow_mut().take();
            if let Some(completed) = inner.state.borrow_mut().complete_current_line() {
                match completed.target {
                    NarrativePresentationTarget::MiloBubble => {
                        inner.milo_popover.popdown();
                        eprintln!("[milo] narrative popover: Hide");
                    }
                    NarrativePresentationTarget::PhotoInspection => {
                        inner.world.hide_inspection_line();
                    }
                }
            }
            Self { inner }.show_next_line();
        });
        self.inner.timeout.replace(Some(timeout));
    }

    fn show_target(&self, presented: PresentedDialogueLine) {
        match presented.target {
            NarrativePresentationTarget::MiloBubble => {
                self.inner.world.close_inspection();
                self.inner.milo_label.set_label(presented.line.text);
                self.inner.milo_popover.popup();
                eprintln!("[milo] narrative popover: Show");
            }
            NarrativePresentationTarget::PhotoInspection => {
                self.inner.world.show_inspection_line(presented.line.text);
            }
        }
    }

    fn hide_target_for_intervention(&self, target: Option<NarrativePresentationTarget>) {
        match target {
            Some(NarrativePresentationTarget::MiloBubble) => {
                self.inner.milo_popover.popdown();
                eprintln!("[milo] narrative popover: Hide");
            }
            Some(NarrativePresentationTarget::PhotoInspection) => {
                self.inner.world.close_inspection();
            }
            None => {}
        }
    }
}

fn log_browser_command_result(result: BrowserCommandResult) {
    eprintln!("[milo] browser command result: {result}");
}

fn plan_intervention_response(response: InterventionResponse) -> InterventionResponsePlan {
    match response {
        InterventionResponse::TakeBreak => InterventionResponsePlan {
            record_break_accepted: true,
            browser_command: Some(BrowserCommand::CloseActiveDistractionTab),
        },
        InterventionResponse::KeepScrolling => InterventionResponsePlan {
            record_break_accepted: false,
            browser_command: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn dialogue_text(dialogue: DialogueSequence) -> Vec<&'static str> {
        dialogue.into_lines().map(|line| line.text).collect()
    }

    fn presentation(
        kind: NarrativeSequenceKind,
        target: NarrativePresentationTarget,
        lines: &[&'static str],
    ) -> NarrativePresentation {
        NarrativePresentation {
            kind,
            target,
            dialogue: DialogueSequence::new(lines),
        }
    }

    fn presented_line(pump: NarrativePump) -> (PresentedDialogueLine, bool) {
        match pump {
            NarrativePump::Present { line, is_new } => (line, is_new),
            other => panic!("expected a presentable narrative line, got {other:?}"),
        }
    }

    #[test]
    fn take_break_narrative_is_planned_independently_from_browser_delivery() {
        assert_eq!(
            plan_intervention_response(InterventionResponse::TakeBreak),
            InterventionResponsePlan {
                record_break_accepted: true,
                browser_command: Some(BrowserCommand::CloseActiveDistractionTab),
            }
        );
        assert_eq!(
            plan_intervention_response(InterventionResponse::KeepScrolling),
            InterventionResponsePlan {
                record_break_accepted: false,
                browser_command: None,
            }
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
    fn transparent_companion_is_the_default_and_explicit_alias() {
        assert_eq!(
            PresentationMode::from_arguments(&arguments(&["milo"])),
            PresentationMode::TransparentCompanion
        );
        assert_eq!(
            PresentationMode::from_arguments(&arguments(&["milo", "--room", "--transparent"])),
            PresentationMode::TransparentCompanion
        );
        assert_eq!(
            PresentationMode::TransparentCompanion.window_size(),
            (TRANSPARENT_WINDOW_WIDTH, TRANSPARENT_WINDOW_HEIGHT)
        );
        assert_eq!(
            PresentationMode::TransparentCompanion.milo_display_size(),
            TRANSPARENT_MILO_DISPLAY_SIZE
        );
    }

    #[test]
    fn room_argument_selects_the_compact_room_layout() {
        let mode = PresentationMode::from_arguments(&arguments(&["milo", "--room"]));

        assert_eq!(mode, PresentationMode::Room);
        assert_eq!(
            mode.window_size(),
            (ROOM_VIEWPORT_WIDTH, ROOM_VIEWPORT_HEIGHT)
        );
        assert_eq!(mode.milo_display_size(), ROOM_MILO_DISPLAY_SIZE);
    }

    #[test]
    fn break_accepted_queued_during_intervention_resumes_without_loss() {
        let mut state = NarrativePresentationState::default();
        assert!(state.set_intervention_visible(true));
        assert!(state.enqueue(presentation(
            NarrativeSequenceKind::BreakAccepted,
            NarrativePresentationTarget::MiloBubble,
            &["Good.", "...I mean, I'll be fine."],
        )));

        assert_eq!(state.pump(), NarrativePump::SuppressedByIntervention);
        assert_eq!(state.queued_lines.len(), 2);

        assert!(state.set_intervention_visible(false));
        let (first, is_new) = presented_line(state.pump());
        assert!(is_new);
        assert_eq!(first.line.text, "Good.");

        assert_eq!(state.complete_current_line(), Some(first));
        let (second, is_new) = presented_line(state.pump());
        assert!(is_new);
        assert_eq!(second.line.text, "...I mean, I'll be fine.");
    }

    #[test]
    fn interrupted_line_resumes_without_restarting_or_advancing() {
        let mut state = NarrativePresentationState::default();
        state.enqueue(presentation(
            NarrativeSequenceKind::BecameConcerned,
            NarrativePresentationTarget::MiloBubble,
            &["current", "next"],
        ));
        let (current, is_new) = presented_line(state.pump());
        assert!(is_new);

        state.set_intervention_visible(true);
        assert_eq!(state.pump(), NarrativePump::SuppressedByIntervention);
        state.set_intervention_visible(false);

        let (resumed, is_new) = presented_line(state.pump());
        assert!(!is_new);
        assert_eq!(resumed, current);
        state.complete_current_line();
        let (next, is_new) = presented_line(state.pump());
        assert!(is_new);
        assert_eq!(next.line.text, "next");
    }

    #[test]
    fn photo_inspection_is_a_target_not_a_suppression_reason() {
        let mut state = NarrativePresentationState::default();
        state.enqueue(presentation(
            NarrativeSequenceKind::EliPhotoInspection,
            NarrativePresentationTarget::PhotoInspection,
            &["That's me."],
        ));

        let (line, is_new) = presented_line(state.pump());
        assert!(is_new);
        assert_eq!(line.target, NarrativePresentationTarget::PhotoInspection);
        assert_eq!(line.line.text, "That's me.");
    }

    #[test]
    fn intervention_suppresses_then_resumes_photo_inspection_target() {
        let mut state = NarrativePresentationState::default();
        state.enqueue(presentation(
            NarrativeSequenceKind::EliPhotoInspection,
            NarrativePresentationTarget::PhotoInspection,
            &["That's me.", "And that's Eli."],
        ));
        let (current, _) = presented_line(state.pump());

        state.set_intervention_visible(true);
        assert_eq!(state.pump(), NarrativePump::SuppressedByIntervention);
        state.set_intervention_visible(false);

        let (resumed, is_new) = presented_line(state.pump());
        assert!(!is_new);
        assert_eq!(resumed, current);
        assert_eq!(resumed.target, NarrativePresentationTarget::PhotoInspection);
    }

    #[test]
    fn mixed_targets_preserve_queue_order() {
        let mut state = NarrativePresentationState::default();
        state.enqueue(presentation(
            NarrativeSequenceKind::FirstLaunch,
            NarrativePresentationTarget::MiloBubble,
            &["first"],
        ));
        state.enqueue(presentation(
            NarrativeSequenceKind::EliPhotoInspection,
            NarrativePresentationTarget::PhotoInspection,
            &["second"],
        ));

        let (first, _) = presented_line(state.pump());
        assert_eq!(first.line.text, "first");
        assert_eq!(first.target, NarrativePresentationTarget::MiloBubble);
        state.complete_current_line();

        let (second, _) = presented_line(state.pump());
        assert_eq!(second.line.text, "second");
        assert_eq!(second.target, NarrativePresentationTarget::PhotoInspection);
    }

    #[test]
    fn world_events_map_to_the_expected_dialogue() {
        let appearance = dialogue_for_world_event(WorldEvent::EliPhotoAppeared).unwrap();
        assert_eq!(appearance.kind, NarrativeSequenceKind::EliPhotoAppeared);
        assert_eq!(appearance.target, NarrativePresentationTarget::MiloBubble);
        assert_eq!(
            dialogue_text(appearance.dialogue),
            ["I left it here.", "Thought you might want to see it."]
        );

        let inspection =
            dialogue_for_world_event(WorldEvent::ObjectInspected(WorldObject::EliPhoto)).unwrap();
        assert_eq!(inspection.kind, NarrativeSequenceKind::EliPhotoInspection);
        assert_eq!(
            inspection.target,
            NarrativePresentationTarget::PhotoInspection
        );
        assert_eq!(
            dialogue_text(inspection.dialogue),
            [
                "That's me.",
                "And that's Eli.",
                "We used to play with that all the time.",
                "...I haven't seen him in a long time."
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
    visible: Rc<Cell<bool>>,
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
            visible: Rc::new(Cell::new(false)),
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

    fn is_visible(&self) -> bool {
        self.visible.get()
    }

    fn connect_closed<F>(&self, handler: F)
    where
        F: Fn() + 'static,
    {
        let visible = Rc::clone(&self.visible);
        self.popover.connect_closed(move |_| {
            visible.set(false);
            handler();
        });
    }

    fn present(&self, presentation: InterventionPresentation) {
        match presentation {
            InterventionPresentation::Show(intervention) => {
                self.visible.set(true);
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
