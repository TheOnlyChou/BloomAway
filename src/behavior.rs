use crate::activity::ActivityCategory;
use crate::animation::{MiloAnimator, MiloState};
use crate::distraction::{DistractionEvent, FIRST_THRESHOLD_SECONDS, SECOND_THRESHOLD_SECONDS};
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use gtk4::glib;
use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
const PLAY_WITH_YARN_LOOPS: usize = 3;
pub const COZY_ACTIVITY_MIN_DELAY_SECONDS: u64 = 120;
pub const COZY_ACTIVITY_MAX_DELAY_SECONDS: u64 = 300;

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
enum CozyActivity {
    PlayWithYarn,
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
    cozy_activity: Option<CozyActivity>,
    temporary_generation: u64,
    distraction_severity: DistractionSeverity,
    intervention_visible: bool,
    narrative_active: bool,
    current: MiloState,
}

impl BehaviorState {
    fn new() -> Self {
        Self {
            system_idle: false,
            curious_reaction: None,
            cozy_activity: None,
            temporary_generation: 0,
            distraction_severity: DistractionSeverity::None,
            intervention_visible: false,
            narrative_active: false,
            current: MiloState::Idle,
        }
    }

    fn system_idle(&mut self) -> Option<StateTransition> {
        self.invalidate_temporary_state();
        self.system_idle = true;
        self.transition_to_automatic_state()
    }

    fn resume(&mut self) -> (u64, Option<StateTransition>) {
        self.invalidate_temporary_state();
        self.system_idle = false;
        self.curious_reaction = Some(CuriousReaction::Resume);
        (
            self.temporary_generation,
            self.transition_to_automatic_state(),
        )
    }

    fn start_app_switch_reaction(&mut self) -> (u64, Option<StateTransition>) {
        self.invalidate_temporary_state();
        self.curious_reaction = Some(CuriousReaction::AppSwitch);
        (
            self.temporary_generation,
            self.transition_to_automatic_state(),
        )
    }

    fn finish_temporary_reaction(&mut self, generation: u64) -> Option<StateTransition> {
        if self.temporary_generation != generation {
            return None;
        }

        self.curious_reaction = None;
        self.transition_to_automatic_state()
    }

    fn start_autonomous_play_with_yarn(&mut self) -> Option<(StateTransition, u64)> {
        if !self.can_start_cozy_activity() {
            return None;
        }

        self.invalidate_temporary_state();
        self.cozy_activity = Some(CozyActivity::PlayWithYarn);
        let generation = self.temporary_generation;
        let transition = self
            .transition_to_state(MiloState::PlayWithYarn)
            .expect("calm Idle must transition to PlayWithYarn");
        Some((transition, generation))
    }

    fn finish_play_with_yarn(&mut self, generation: u64) -> Option<StateTransition> {
        if self.temporary_generation != generation
            || self.cozy_activity != Some(CozyActivity::PlayWithYarn)
        {
            return None;
        }

        self.cozy_activity = None;
        self.transition_to_automatic_state()
    }

    fn handle_distraction_event(&mut self, event: DistractionEvent) -> Option<StateTransition> {
        if self.curious_reaction == Some(CuriousReaction::AppSwitch) || self.cozy_activity.is_some()
        {
            self.invalidate_temporary_state();
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
        self.invalidate_temporary_state();
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

    fn has_autonomous_cozy_activity(&self) -> bool {
        self.cozy_activity.is_some()
    }

    fn can_start_cozy_activity(&self) -> bool {
        !self.system_idle
            && self.curious_reaction.is_none()
            && self.distraction_severity == DistractionSeverity::None
            && !self.intervention_visible
            && !self.narrative_active
            && self.automatic_state() == MiloState::Idle
            && self.current == MiloState::Idle
    }

    fn set_intervention_visible(&mut self, visible: bool) -> Option<StateTransition> {
        self.intervention_visible = visible;
        if visible && self.interrupt_cozy_activity() {
            self.transition_to_automatic_state()
        } else {
            None
        }
    }

    fn set_narrative_active(&mut self, active: bool) -> Option<StateTransition> {
        self.narrative_active = active;
        if active && self.interrupt_cozy_activity() {
            self.transition_to_automatic_state()
        } else {
            None
        }
    }

    fn invalidate_temporary_state(&mut self) {
        self.temporary_generation = self.temporary_generation.wrapping_add(1);
        self.curious_reaction = None;
        self.cozy_activity = None;
    }

    fn interrupt_cozy_activity(&mut self) -> bool {
        if self.cozy_activity.take().is_none() {
            return false;
        }

        self.temporary_generation = self.temporary_generation.wrapping_add(1);
        true
    }

    fn transition_to_state(&mut self, next: MiloState) -> Option<StateTransition> {
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

    fn transition_to_automatic_state(&mut self) -> Option<StateTransition> {
        self.transition_to_state(self.automatic_state())
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

struct CozyDelayGenerator {
    state: u64,
}

impl CozyDelayGenerator {
    fn from_system_time() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seed = timestamp as u64
            ^ (timestamp >> u64::BITS) as u64
            ^ u64::from(std::process::id()).rotate_left(32);
        Self::seeded(seed)
    }

    fn seeded(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }

    fn next_delay(&mut self) -> Duration {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;

        let delay_range = COZY_ACTIVITY_MAX_DELAY_SECONDS - COZY_ACTIVITY_MIN_DELAY_SECONDS + 1;
        Duration::from_secs(COZY_ACTIVITY_MIN_DELAY_SECONDS + value % delay_range)
    }
}

struct CozyActivityScheduler {
    timeout: RefCell<Option<glib::SourceId>>,
    delay_generator: RefCell<CozyDelayGenerator>,
}

impl CozyActivityScheduler {
    fn new() -> Self {
        Self {
            timeout: RefCell::new(None),
            delay_generator: RefCell::new(CozyDelayGenerator::from_system_time()),
        }
    }
}

struct BehaviorInner {
    animator: MiloAnimator,
    state: RefCell<BehaviorState>,
    curious_timeout: RefCell<Option<glib::SourceId>>,
    cozy_scheduler: CozyActivityScheduler,
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
                cozy_scheduler: CozyActivityScheduler::new(),
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

    pub fn start_cozy_activity_scheduler(&self) {
        self.schedule_next_cozy_opportunity();
    }

    pub fn cycle_debug_state(&self) -> MiloState {
        self.clear_curious_timeout();
        let interrupted = self.inner.state.borrow().has_autonomous_cozy_activity();
        let transition = self.inner.state.borrow_mut().cycle_debug_state();
        self.apply_transition(Some(transition));
        self.handle_cozy_interruption(interrupted);
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
        let interrupted = self.inner.state.borrow().has_autonomous_cozy_activity();
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
        self.handle_cozy_interruption(interrupted);
    }

    pub fn set_intervention_visible(&self, visible: bool) {
        let interrupted = visible && self.inner.state.borrow().has_autonomous_cozy_activity();
        let transition = self
            .inner
            .state
            .borrow_mut()
            .set_intervention_visible(visible);
        self.apply_transition(transition);
        self.handle_cozy_interruption(interrupted);
    }

    pub fn set_narrative_active(&self, active: bool) {
        let interrupted = active && self.inner.state.borrow().has_autonomous_cozy_activity();
        let transition = self.inner.state.borrow_mut().set_narrative_active(active);
        self.apply_transition(transition);
        self.handle_cozy_interruption(interrupted);
    }

    fn handle_activity_event(&self, event: ActivityEvent) {
        self.clear_curious_timeout();
        match event {
            ActivityEvent::Idled => {
                let interrupted = self.inner.state.borrow().has_autonomous_cozy_activity();
                let transition = self.inner.state.borrow_mut().system_idle();
                self.apply_transition(transition);
                self.handle_cozy_interruption(interrupted);
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
        let interrupted = self.inner.state.borrow().has_autonomous_cozy_activity();
        let (generation, transition) = match reaction {
            CuriousReaction::Resume => self.inner.state.borrow_mut().resume(),
            CuriousReaction::AppSwitch => self.inner.state.borrow_mut().start_app_switch_reaction(),
        };
        self.apply_transition(transition);
        self.handle_cozy_interruption(interrupted);

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

    fn play_autonomous_yarn(&self, generation: u64) {
        let weak_inner = Rc::downgrade(&self.inner);
        self.inner
            .animator
            .play_loops(MiloState::PlayWithYarn, PLAY_WITH_YARN_LOOPS, move || {
                let Some(inner) = weak_inner.upgrade() else {
                    return;
                };
                let transition = inner.state.borrow_mut().finish_play_with_yarn(generation);
                if let Some(transition) = transition {
                    eprintln!("[milo] cozy activity completed: PlayWithYarn");
                    inner.animator.set_state(transition.current);
                    Self { inner }.schedule_next_cozy_opportunity();
                }
            });
    }

    fn schedule_next_cozy_opportunity(&self) {
        if self.inner.cozy_scheduler.timeout.borrow().is_some() {
            return;
        }

        let delay = self
            .inner
            .cozy_scheduler
            .delay_generator
            .borrow_mut()
            .next_delay();
        eprintln!("[milo] cozy activity scheduled: PlayWithYarn");

        let weak_inner = Rc::downgrade(&self.inner);
        let timeout = glib::timeout_add_local_once(delay, move || {
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };
            inner.cozy_scheduler.timeout.borrow_mut().take();
            Self { inner }.handle_cozy_opportunity();
        });
        self.inner.cozy_scheduler.timeout.replace(Some(timeout));
    }

    fn handle_cozy_opportunity(&self) {
        let start = self
            .inner
            .state
            .borrow_mut()
            .start_autonomous_play_with_yarn();
        let Some((_transition, generation)) = start else {
            self.schedule_next_cozy_opportunity();
            return;
        };

        eprintln!("[milo] cozy activity started: PlayWithYarn");
        self.play_autonomous_yarn(generation);
    }

    fn handle_cozy_interruption(&self, interrupted: bool) {
        if !interrupted {
            return;
        }

        eprintln!("[milo] cozy activity interrupted: PlayWithYarn");
        self.schedule_next_cozy_opportunity();
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

    fn start_autonomous_yarn(state: &mut BehaviorState) -> u64 {
        let (transition, generation) = state.start_autonomous_play_with_yarn().unwrap();
        assert_eq!(transition.current, MiloState::PlayWithYarn);
        generation
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

    #[test]
    fn play_with_yarn_may_start_from_calm_authoritative_idle() {
        let mut state = BehaviorState::new();
        let generation = start_autonomous_yarn(&mut state);

        assert_eq!(state.current, MiloState::PlayWithYarn);
        assert_eq!(state.cozy_activity, Some(CozyActivity::PlayWithYarn));
        assert_eq!(
            state.finish_play_with_yarn(generation),
            Some(StateTransition {
                previous: MiloState::PlayWithYarn,
                current: MiloState::Idle,
            })
        );
    }

    #[test]
    fn play_with_yarn_completion_recomputes_authoritative_state() {
        let mut state = BehaviorState::new();
        let generation = start_autonomous_yarn(&mut state);
        state.distraction_severity = DistractionSeverity::Concerned;

        assert_eq!(
            state.finish_play_with_yarn(generation),
            Some(StateTransition {
                previous: MiloState::PlayWithYarn,
                current: MiloState::Concerned,
            })
        );
    }

    #[test]
    fn play_with_yarn_cannot_start_while_system_idle() {
        let mut state = BehaviorState::new();
        state.system_idle();

        assert!(state.start_autonomous_play_with_yarn().is_none());
        assert_eq!(state.current, MiloState::Sleeping);
    }

    #[test]
    fn play_with_yarn_cannot_start_during_curious_or_concerned_distraction() {
        for seconds in [FIRST_THRESHOLD_SECONDS, SECOND_THRESHOLD_SECONDS] {
            let mut state = BehaviorState::new();
            state.handle_distraction_event(started());
            state.handle_distraction_event(threshold(seconds));

            assert!(state.start_autonomous_play_with_yarn().is_none());
            assert_eq!(
                state.current,
                if seconds == FIRST_THRESHOLD_SECONDS {
                    MiloState::Curious
                } else {
                    MiloState::Concerned
                }
            );
        }
    }

    #[test]
    fn play_with_yarn_cannot_start_during_an_active_distraction() {
        let mut state = BehaviorState::new();
        state.handle_distraction_event(started());

        assert!(state.start_autonomous_play_with_yarn().is_none());
        assert_eq!(state.current, MiloState::Idle);
    }

    #[test]
    fn play_with_yarn_cannot_start_during_a_resume_reaction() {
        let mut state = BehaviorState::new();
        state.system_idle();
        state.resume();

        assert!(state.start_autonomous_play_with_yarn().is_none());
        assert_eq!(state.current, MiloState::Curious);
    }

    #[test]
    fn play_with_yarn_cannot_start_during_an_app_switch_reaction() {
        let mut state = BehaviorState::new();
        state.start_app_switch_reaction();

        assert!(state.start_autonomous_play_with_yarn().is_none());
        assert_eq!(state.current, MiloState::Curious);
    }

    #[test]
    fn play_with_yarn_waits_for_intervention_and_narrative_ui() {
        let mut state = BehaviorState::new();
        state.set_intervention_visible(true);
        assert!(state.start_autonomous_play_with_yarn().is_none());

        state.set_intervention_visible(false);
        state.set_narrative_active(true);
        assert!(state.start_autonomous_play_with_yarn().is_none());

        state.set_narrative_active(false);
        assert!(state.start_autonomous_play_with_yarn().is_some());
    }

    #[test]
    fn system_idle_interrupts_play_with_yarn() {
        let mut state = BehaviorState::new();
        let generation = start_autonomous_yarn(&mut state);

        assert_eq!(state.system_idle().unwrap().current, MiloState::Sleeping);
        assert_eq!(state.finish_play_with_yarn(generation), None);
        assert_eq!(state.current, MiloState::Sleeping);
    }

    #[test]
    fn distraction_concerned_interrupts_play_with_yarn() {
        let mut state = BehaviorState::new();
        let generation = start_autonomous_yarn(&mut state);

        assert_eq!(
            state
                .handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS))
                .unwrap()
                .current,
            MiloState::Concerned
        );
        assert_eq!(state.finish_play_with_yarn(generation), None);
        assert_eq!(state.current, MiloState::Concerned);
    }

    #[test]
    fn stale_play_with_yarn_completion_cannot_overwrite_concerned() {
        let mut state = BehaviorState::new();
        let play_with_yarn_generation = start_autonomous_yarn(&mut state);
        state.handle_distraction_event(threshold(SECOND_THRESHOLD_SECONDS));

        assert_eq!(state.finish_play_with_yarn(play_with_yarn_generation), None);
        assert_eq!(state.current, MiloState::Concerned);
    }

    #[test]
    fn cozy_delays_are_varied_and_stay_within_development_bounds() {
        let mut generator = CozyDelayGenerator::seeded(42);
        let delays = (0..16)
            .map(|_| generator.next_delay().as_secs())
            .collect::<Vec<_>>();

        assert!(delays.iter().all(|delay| {
            (COZY_ACTIVITY_MIN_DELAY_SECONDS..=COZY_ACTIVITY_MAX_DELAY_SECONDS).contains(delay)
        }));
        assert!(delays.windows(2).any(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn completion_policy_produces_another_randomized_opportunity() {
        let mut generator = CozyDelayGenerator::seeded(7);
        let first_opportunity = generator.next_delay();
        let after_completion = generator.next_delay();

        assert_ne!(first_opportunity, after_completion);
        assert!(after_completion >= Duration::from_secs(COZY_ACTIVITY_MIN_DELAY_SECONDS));
        assert!(after_completion <= Duration::from_secs(COZY_ACTIVITY_MAX_DELAY_SECONDS));
    }
}
