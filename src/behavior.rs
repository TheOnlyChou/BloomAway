use crate::activity::ActivityCategory;
use crate::animation::{MiloAnimator, MiloState};
use crate::distraction::{DistractionEvent, FIRST_THRESHOLD_SECONDS, SECOND_THRESHOLD_SECONDS};
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use gtk4::glib;
use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

pub const DEVELOPMENT_IDLE_TIMEOUT_SECONDS: u32 = 60;
pub const CURIOUS_AFTER_RESUME_MS: u64 = 3_000;
pub const APP_SWITCH_CURIOUS_MS: u64 = 1_500;

#[derive(Clone, Copy, Debug)]
enum ActivityEvent {
    Idled,
    Resumed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CuriousReaction {
    Resume,
    AppSwitch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DistractionSeverity {
    None,
    Active,
    Curious,
    Concerned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StateTransition {
    previous: MiloState,
    current: MiloState,
}

struct BehaviorState {
    system_idle: bool,
    curious_reaction: Option<CuriousReaction>,
    reaction_generation: u64,
    distraction_severity: DistractionSeverity,
    current: MiloState,
}

impl BehaviorState {
    fn new() -> Self {
        Self {
            system_idle: false,
            curious_reaction: None,
            reaction_generation: 0,
            distraction_severity: DistractionSeverity::None,
            current: MiloState::Idle,
        }
    }

    fn system_idle(&mut self) -> Option<StateTransition> {
        self.invalidate_temporary_reaction();
        self.system_idle = true;
        self.transition_to_automatic_state()
    }

    fn resume(&mut self) -> (u64, Option<StateTransition>) {
        self.invalidate_temporary_reaction();
        self.system_idle = false;
        self.curious_reaction = Some(CuriousReaction::Resume);
        (
            self.reaction_generation,
            self.transition_to_automatic_state(),
        )
    }

    fn start_app_switch_reaction(&mut self) -> (u64, Option<StateTransition>) {
        self.invalidate_temporary_reaction();
        self.curious_reaction = Some(CuriousReaction::AppSwitch);
        (
            self.reaction_generation,
            self.transition_to_automatic_state(),
        )
    }

    fn finish_temporary_reaction(&mut self, generation: u64) -> Option<StateTransition> {
        if self.reaction_generation != generation {
            return None;
        }

        self.curious_reaction = None;
        self.transition_to_automatic_state()
    }

    fn handle_distraction_event(&mut self, event: DistractionEvent) -> Option<StateTransition> {
        if self.curious_reaction == Some(CuriousReaction::AppSwitch) {
            self.invalidate_temporary_reaction();
        }

        match event {
            DistractionEvent::Started(_) => {
                self.distraction_severity = DistractionSeverity::Active;
            }
            DistractionEvent::Threshold { seconds, .. } if seconds == FIRST_THRESHOLD_SECONDS => {
                self.distraction_severity = DistractionSeverity::Curious;
            }
            DistractionEvent::Threshold { seconds, .. } if seconds == SECOND_THRESHOLD_SECONDS => {
                self.distraction_severity = DistractionSeverity::Concerned;
            }
            DistractionEvent::Threshold { .. } => {}
            DistractionEvent::Ended { .. } => {
                self.distraction_severity = DistractionSeverity::None;
            }
        }

        self.transition_to_automatic_state()
    }

    fn cycle_debug_state(&mut self) -> StateTransition {
        self.invalidate_temporary_reaction();
        let previous = self.current;
        self.current = self.current.next();
        StateTransition {
            previous,
            current: self.current,
        }
    }

    fn app_switch_reaction_allowed(&self) -> bool {
        !self.system_idle
            && self.curious_reaction != Some(CuriousReaction::Resume)
            && self.distraction_severity == DistractionSeverity::None
    }

    fn has_app_switch_reaction(&self) -> bool {
        self.curious_reaction == Some(CuriousReaction::AppSwitch)
    }

    fn invalidate_temporary_reaction(&mut self) {
        self.reaction_generation = self.reaction_generation.wrapping_add(1);
        self.curious_reaction = None;
    }

    fn transition_to_automatic_state(&mut self) -> Option<StateTransition> {
        let next = self.automatic_state();
        if next == self.current {
            return None;
        }

        let transition = StateTransition {
            previous: self.current,
            current: next,
        };
        self.current = next;
        Some(transition)
    }

    fn automatic_state(&self) -> MiloState {
        if self.system_idle {
            return MiloState::Sleeping;
        }
        if self.curious_reaction == Some(CuriousReaction::Resume) {
            return MiloState::Curious;
        }

        match self.distraction_severity {
            DistractionSeverity::Active => MiloState::Idle,
            DistractionSeverity::Curious => MiloState::Curious,
            DistractionSeverity::Concerned => MiloState::Concerned,
            DistractionSeverity::None => {
                if self.curious_reaction == Some(CuriousReaction::AppSwitch) {
                    MiloState::Curious
                } else {
                    MiloState::Idle
                }
            }
        }
    }
}

struct BehaviorInner {
    animator: MiloAnimator,
    state: RefCell<BehaviorState>,
    curious_timeout: RefCell<Option<glib::SourceId>>,
}

#[derive(Clone)]
pub struct BehaviorController {
    inner: Rc<BehaviorInner>,
}

impl BehaviorController {
    pub fn new(animator: MiloAnimator) -> Self {
        Self {
            inner: Rc::new(BehaviorInner {
                animator,
                state: RefCell::new(BehaviorState::new()),
                curious_timeout: RefCell::new(None),
            }),
        }
    }

    pub fn start_idle_monitor<F>(&self, mut on_system_idle_change: F)
    where
        F: FnMut(bool) + 'static,
    {
        let (sender, mut receiver) = unbounded();
        let controller = self.clone();

        glib::MainContext::default().spawn_local(async move {
            while let Some(event) = receiver.next().await {
                controller.handle_activity_event(event);
                on_system_idle_change(matches!(event, ActivityEvent::Idled));
            }
        });

        if let Err(error) = std::thread::Builder::new()
            .name("milo-idle-notify".into())
            .spawn(move || {
                if let Err(error) = run_idle_listener(sender) {
                    eprintln!("Milo automatic idle behavior unavailable: {error}");
                }
            })
        {
            eprintln!(
                "Milo automatic idle behavior unavailable: failed to start listener: {error}"
            );
        }
    }

    pub fn cycle_debug_state(&self) -> MiloState {
        self.clear_curious_timeout();
        let transition = self.inner.state.borrow_mut().cycle_debug_state();
        self.apply_transition(Some(transition));
        transition.current
    }

    pub fn handle_category_change(&self, previous: ActivityCategory, current: ActivityCategory) {
        if previous == current || !self.inner.state.borrow().app_switch_reaction_allowed() {
            return;
        }

        eprintln!("[milo] context changed: {previous} -> {current}");
        eprintln!("[milo] reaction: Curious");
        self.start_curious_reaction(CuriousReaction::AppSwitch, APP_SWITCH_CURIOUS_MS);
    }

    pub fn handle_distraction_event(&self, event: DistractionEvent) {
        if self.inner.state.borrow().has_app_switch_reaction() {
            self.clear_curious_timeout();
        }
        let transition = self
            .inner
            .state
            .borrow_mut()
            .handle_distraction_event(event);
        if let Some(transition) = transition {
            eprintln!(
                "[milo] distraction state: {:?} -> {:?}",
                transition.previous, transition.current
            );
        }
        self.apply_transition(transition);
    }

    fn handle_activity_event(&self, event: ActivityEvent) {
        self.clear_curious_timeout();
        match event {
            ActivityEvent::Idled => {
                let transition = self.inner.state.borrow_mut().system_idle();
                self.apply_transition(transition);
                eprintln!("Milo automatic state: Sleeping (user idle)");
            }
            ActivityEvent::Resumed => {
                eprintln!("Milo automatic state: Curious (user resumed)");
                self.start_curious_reaction(CuriousReaction::Resume, CURIOUS_AFTER_RESUME_MS);
            }
        }
    }

    fn start_curious_reaction(&self, reaction: CuriousReaction, duration_ms: u64) {
        self.clear_curious_timeout();
        let (generation, transition) = match reaction {
            CuriousReaction::Resume => self.inner.state.borrow_mut().resume(),
            CuriousReaction::AppSwitch => self.inner.state.borrow_mut().start_app_switch_reaction(),
        };
        self.apply_transition(transition);

        let weak_inner = Rc::downgrade(&self.inner);
        let timeout = glib::timeout_add_local_once(Duration::from_millis(duration_ms), move || {
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            inner.curious_timeout.borrow_mut().take();
            let transition = inner
                .state
                .borrow_mut()
                .finish_temporary_reaction(generation);
            if let Some(transition) = transition {
                inner.animator.set_state(transition.current);
                if reaction == CuriousReaction::Resume {
                    eprintln!(
                        "Milo automatic state: {:?} (resume reaction complete)",
                        transition.current
                    );
                }
            }
        });
        self.inner.curious_timeout.replace(Some(timeout));
    }

    fn clear_curious_timeout(&self) {
        if let Some(timeout) = self.inner.curious_timeout.borrow_mut().take() {
            timeout.remove();
        }
    }

    fn apply_transition(&self, transition: Option<StateTransition>) {
        if let Some(transition) = transition {
            self.inner.animator.set_state(transition.current);
        }
    }
}

struct IdleListenerState {
    sender: UnboundedSender<ActivityEvent>,
    receiver_closed: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for IdleListenerState {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(IdleListenerState: ignore wl_seat::WlSeat);
delegate_noop!(IdleListenerState: ExtIdleNotifierV1);

impl Dispatch<ExtIdleNotificationV1, ()> for IdleListenerState {
    fn event(
        state: &mut Self,
        _notification: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        let activity_event = match event {
            ext_idle_notification_v1::Event::Idled => ActivityEvent::Idled,
            ext_idle_notification_v1::Event::Resumed => ActivityEvent::Resumed,
            _ => return,
        };

        if state.sender.unbounded_send(activity_event).is_err() {
            state.receiver_closed = true;
        }
    }
}

fn run_idle_listener(
    sender: UnboundedSender<ActivityEvent>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let connection = Connection::connect_to_env()
        .map_err(|error| format!("could not connect to the Wayland compositor: {error}"))?;
    let (globals, mut event_queue) = registry_queue_init::<IdleListenerState>(&connection)
        .map_err(|error| format!("could not read Wayland globals: {error}"))?;
    let queue_handle = event_queue.handle();

    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&queue_handle, 1..=1, ())
        .map_err(|error| format!("no usable wl_seat is available: {error}"))?;
    let notifier = globals
        .bind::<ExtIdleNotifierV1, _, _>(&queue_handle, 1..=2, ())
        .map_err(|error| format!("ext-idle-notify-v1 is not supported: {error}"))?;

    let timeout_ms = DEVELOPMENT_IDLE_TIMEOUT_SECONDS * 1_000;
    let _notification = if notifier.version() >= 2 {
        notifier.get_input_idle_notification(timeout_ms, &seat, &queue_handle, ())
    } else {
        notifier.get_idle_notification(timeout_ms, &seat, &queue_handle, ())
    };
    event_queue.flush()?;

    let mut state = IdleListenerState {
        sender,
        receiver_closed: false,
    };
    while !state.receiver_closed {
        event_queue.blocking_dispatch(&mut state)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> DistractionEvent {
        DistractionEvent::Started(crate::distraction::DistractionKind::YouTubeShorts)
    }

    fn threshold(seconds: u64) -> DistractionEvent {
        DistractionEvent::Threshold {
            kind: crate::distraction::DistractionKind::YouTubeShorts,
            seconds,
        }
    }

    fn ended() -> DistractionEvent {
        DistractionEvent::Ended {
            kind: crate::distraction::DistractionKind::YouTubeShorts,
            elapsed: Duration::from_secs(21),
        }
    }

    #[test]
    fn distraction_severity_progresses_and_thirty_seconds_stays_concerned() {
        let mut state = BehaviorState::new();

        assert_eq!(state.handle_distraction_event(started()), None);
        assert_eq!(state.current, MiloState::Idle);
        assert_eq!(
            state.handle_distraction_event(threshold(FIRST_THRESHOLD_SECONDS)),
            Some(StateTransition {
                previous: MiloState::Idle,
                current: MiloState::Curious,
            })
        );
        assert_eq!(
            state.handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS)),
            Some(StateTransition {
                previous: MiloState::Curious,
                current: MiloState::Concerned,
            })
        );
        assert_eq!(
            state.handle_distraction_event(threshold(crate::distraction::THIRD_THRESHOLD_SECONDS)),
            None
        );
        assert_eq!(state.current, MiloState::Concerned);
    }

    #[test]
    fn ending_sessions_returns_curious_or_concerned_to_idle() {
        for final_threshold in [FIRST_THRESHOLD_SECONDS, SECOND_THRESHOLD_SECONDS] {
            let mut state = BehaviorState::new();
            state.handle_distraction_event(started());
            state.handle_distraction_event(threshold(final_threshold));

            assert_eq!(
                state.handle_distraction_event(ended()).unwrap().current,
                MiloState::Idle
            );
        }
    }

    #[test]
    fn system_idle_overrides_concerned() {
        let mut state = BehaviorState::new();
        state.handle_distraction_event(started());
        state.handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS));

        assert_eq!(state.system_idle().unwrap().current, MiloState::Sleeping);
    }

    #[test]
    fn resume_after_sleeping_is_temporarily_curious_then_idle() {
        let mut state = BehaviorState::new();
        state.system_idle();

        let (generation, resumed) = state.resume();
        assert_eq!(resumed.unwrap().current, MiloState::Curious);
        assert_eq!(
            state.finish_temporary_reaction(generation).unwrap().current,
            MiloState::Idle
        );
    }

    #[test]
    fn new_session_after_concerned_restarts_at_idle() {
        let mut state = BehaviorState::new();
        state.handle_distraction_event(started());
        state.handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS));
        state.handle_distraction_event(ended());

        assert_eq!(state.handle_distraction_event(started()), None);
        assert_eq!(state.current, MiloState::Idle);
        assert_eq!(state.distraction_severity, DistractionSeverity::Active);
    }

    #[test]
    fn switching_distraction_kind_resets_severity() {
        let mut state = BehaviorState::new();
        state.handle_distraction_event(started());
        state.handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS));
        state.handle_distraction_event(ended());
        let transition = state.handle_distraction_event(DistractionEvent::Started(
            crate::distraction::DistractionKind::InstagramReels,
        ));

        assert_eq!(transition, None);
        assert_eq!(state.current, MiloState::Idle);
        assert_eq!(state.distraction_severity, DistractionSeverity::Active);
    }

    #[test]
    fn stale_temporary_timer_cannot_overwrite_concerned() {
        let mut state = BehaviorState::new();
        let (generation, _) = state.start_app_switch_reaction();
        state.handle_distraction_event(started());
        state.handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS));

        assert_eq!(state.finish_temporary_reaction(generation), None);
        assert_eq!(state.current, MiloState::Concerned);
    }

    #[test]
    fn completing_resume_reaction_uses_persistent_distraction_state() {
        let mut state = BehaviorState::new();
        state.system_idle();
        let (generation, _) = state.resume();
        state.handle_distraction_event(started());
        state.handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS));

        assert_eq!(
            state.finish_temporary_reaction(generation).unwrap().current,
            MiloState::Concerned
        );
    }

    #[test]
    fn app_switch_reactions_do_not_override_distraction_severity() {
        let mut state = BehaviorState::new();
        state.handle_distraction_event(started());

        assert!(!state.app_switch_reaction_allowed());
        state.handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS));
        assert!(!state.app_switch_reaction_allowed());
    }
}
