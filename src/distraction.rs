use crate::activity::ActivityCategory;
use gtk4::glib;
use milo::browser::BrowserActivity;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const FIRST_THRESHOLD_SECONDS: u64 = 10;
pub const SECOND_THRESHOLD_SECONDS: u64 = 20;
pub const THIRD_THRESHOLD_SECONDS: u64 = 30;

const THRESHOLDS: [u64; 3] = [
    FIRST_THRESHOLD_SECONDS,
    SECOND_THRESHOLD_SECONDS,
    THIRD_THRESHOLD_SECONDS,
];
const TIMER_CHECK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistractionKind {
    YouTubeShorts,
    InstagramReels,
}

#[derive(Debug)]
struct DistractionSession {
    kind: DistractionKind,
    started_at: Instant,
    next_threshold: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DistractionEvent {
    Started(DistractionKind),
    Threshold {
        kind: DistractionKind,
        seconds: u64,
    },
    Ended {
        kind: DistractionKind,
        elapsed: Duration,
    },
}

#[derive(Debug)]
struct DistractionTracker {
    desktop_activity: Option<ActivityCategory>,
    browser_activity: Option<BrowserActivity>,
    system_idle: bool,
    session: Option<DistractionSession>,
}

impl DistractionTracker {
    fn new() -> Self {
        Self {
            desktop_activity: None,
            browser_activity: None,
            system_idle: false,
            session: None,
        }
    }

    fn update_desktop_activity(
        &mut self,
        activity: ActivityCategory,
        now: Instant,
    ) -> Vec<DistractionEvent> {
        self.desktop_activity = Some(activity);
        self.evaluate_session(now)
    }

    fn update_browser_activity(
        &mut self,
        activity: BrowserActivity,
        now: Instant,
    ) -> Vec<DistractionEvent> {
        self.browser_activity = Some(activity);
        self.evaluate_session(now)
    }

    fn browser_unavailable(&mut self, now: Instant) -> Vec<DistractionEvent> {
        self.browser_activity = None;
        self.evaluate_session(now)
    }

    fn update_system_idle(&mut self, system_idle: bool, now: Instant) -> Vec<DistractionEvent> {
        self.system_idle = system_idle;
        self.evaluate_session(now)
    }

    fn check_thresholds(&mut self, now: Instant) -> Vec<DistractionEvent> {
        let Some(session) = self.session.as_mut() else {
            return Vec::new();
        };

        let elapsed = now.saturating_duration_since(session.started_at);
        let mut events = Vec::new();
        while let Some(&seconds) = THRESHOLDS.get(session.next_threshold) {
            if elapsed < Duration::from_secs(seconds) {
                break;
            }

            events.push(DistractionEvent::Threshold {
                kind: session.kind,
                seconds,
            });
            session.next_threshold += 1;
        }
        events
    }

    fn evaluate_session(&mut self, now: Instant) -> Vec<DistractionEvent> {
        let desired_kind = self.desired_kind();
        if self.session.as_ref().map(|session| session.kind) == desired_kind {
            return self.check_thresholds(now);
        }

        let mut events = self.check_thresholds(now);
        if let Some(session) = self.session.take() {
            events.push(DistractionEvent::Ended {
                kind: session.kind,
                elapsed: now.saturating_duration_since(session.started_at),
            });
        }
        if let Some(kind) = desired_kind {
            self.session = Some(DistractionSession {
                kind,
                started_at: now,
                next_threshold: 0,
            });
            events.push(DistractionEvent::Started(kind));
        }
        events
    }

    fn desired_kind(&self) -> Option<DistractionKind> {
        if self.system_idle || self.desktop_activity != Some(ActivityCategory::Browser) {
            return None;
        }

        match self.browser_activity {
            Some(BrowserActivity::YouTubeShorts) => Some(DistractionKind::YouTubeShorts),
            Some(BrowserActivity::InstagramReels) => Some(DistractionKind::InstagramReels),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct DistractionController {
    tracker: Rc<RefCell<DistractionTracker>>,
    event_handler: DistractionEventHandler,
}

type DistractionEventHandler = Rc<RefCell<Box<dyn FnMut(DistractionEvent)>>>;

impl DistractionController {
    pub fn new<F>(event_handler: F) -> Self
    where
        F: FnMut(DistractionEvent) + 'static,
    {
        Self {
            tracker: Rc::new(RefCell::new(DistractionTracker::new())),
            event_handler: Rc::new(RefCell::new(Box::new(event_handler))),
        }
    }

    pub fn start_threshold_monitor(&self) {
        let controller = self.clone();
        glib::timeout_add_local(TIMER_CHECK_INTERVAL, move || {
            controller.handle_events(
                controller
                    .tracker
                    .borrow_mut()
                    .check_thresholds(Instant::now()),
            );
            glib::ControlFlow::Continue
        });
    }

    pub fn update_desktop_activity(&self, activity: ActivityCategory) {
        let events = self
            .tracker
            .borrow_mut()
            .update_desktop_activity(activity, Instant::now());
        self.handle_events(events);
    }

    pub fn update_browser_activity(&self, activity: BrowserActivity) {
        let events = self
            .tracker
            .borrow_mut()
            .update_browser_activity(activity, Instant::now());
        self.handle_events(events);
    }

    pub fn browser_unavailable(&self) {
        let events = self
            .tracker
            .borrow_mut()
            .browser_unavailable(Instant::now());
        self.handle_events(events);
    }

    pub fn update_system_idle(&self, system_idle: bool) {
        let events = self
            .tracker
            .borrow_mut()
            .update_system_idle(system_idle, Instant::now());
        self.handle_events(events);
    }

    fn handle_events(&self, events: Vec<DistractionEvent>) {
        for event in events {
            match &event {
                DistractionEvent::Started(kind) => {
                    eprintln!("[milo] distraction started: {kind:?}");
                }
                DistractionEvent::Threshold { kind, seconds } => {
                    eprintln!("[milo] distraction {seconds}s: {kind:?}");
                }
                DistractionEvent::Ended { kind, elapsed } => {
                    eprintln!(
                        "[milo] distraction ended: {kind:?} ({:.1}s)",
                        elapsed.as_secs_f64()
                    );
                }
            }
            (self.event_handler.borrow_mut())(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(kind: DistractionKind) -> Vec<DistractionEvent> {
        vec![DistractionEvent::Started(kind)]
    }

    #[test]
    fn browser_and_youtube_shorts_starts_a_session_in_either_update_order() {
        let now = Instant::now();
        let mut desktop_first = DistractionTracker::new();
        assert!(
            desktop_first
                .update_desktop_activity(ActivityCategory::Browser, now)
                .is_empty()
        );
        assert_eq!(
            desktop_first.update_browser_activity(BrowserActivity::YouTubeShorts, now),
            started(DistractionKind::YouTubeShorts)
        );

        let mut browser_first = DistractionTracker::new();
        assert!(
            browser_first
                .update_browser_activity(BrowserActivity::YouTubeShorts, now)
                .is_empty()
        );
        assert_eq!(
            browser_first.update_desktop_activity(ActivityCategory::Browser, now),
            started(DistractionKind::YouTubeShorts)
        );
    }

    #[test]
    fn terminal_and_youtube_shorts_does_not_start_a_session() {
        let now = Instant::now();
        let mut tracker = DistractionTracker::new();
        tracker.update_browser_activity(BrowserActivity::YouTubeShorts, now);

        assert!(
            tracker
                .update_desktop_activity(ActivityCategory::Terminal, now)
                .is_empty()
        );
        assert!(tracker.session.is_none());
    }

    #[test]
    fn normal_browser_activities_never_start_a_session() {
        let now = Instant::now();
        for browser_activity in [
            BrowserActivity::YouTube,
            BrowserActivity::Instagram,
            BrowserActivity::Other,
        ] {
            let mut tracker = DistractionTracker::new();
            tracker.update_desktop_activity(ActivityCategory::Browser, now);

            assert!(
                tracker
                    .update_browser_activity(browser_activity, now)
                    .is_empty()
            );
            assert!(tracker.session.is_none());
        }
    }

    #[test]
    fn leaving_browser_ends_a_shorts_session() {
        let started_at = Instant::now();
        let mut tracker = active_session(BrowserActivity::YouTubeShorts, started_at);
        let ended_at = started_at + Duration::from_secs(8);

        assert_eq!(
            tracker.update_desktop_activity(ActivityCategory::Terminal, ended_at),
            vec![DistractionEvent::Ended {
                kind: DistractionKind::YouTubeShorts,
                elapsed: Duration::from_secs(8),
            }]
        );
    }

    #[test]
    fn normal_youtube_ends_a_shorts_session() {
        let started_at = Instant::now();
        let mut tracker = active_session(BrowserActivity::YouTubeShorts, started_at);

        assert_eq!(
            tracker.update_browser_activity(
                BrowserActivity::YouTube,
                started_at + Duration::from_secs(4)
            ),
            vec![DistractionEvent::Ended {
                kind: DistractionKind::YouTubeShorts,
                elapsed: Duration::from_secs(4),
            }]
        );
    }

    #[test]
    fn changing_distraction_kind_ends_then_starts() {
        let started_at = Instant::now();
        let mut tracker = active_session(BrowserActivity::YouTubeShorts, started_at);
        let changed_at = started_at + Duration::from_secs(6);

        assert_eq!(
            tracker.update_browser_activity(BrowserActivity::InstagramReels, changed_at),
            vec![
                DistractionEvent::Ended {
                    kind: DistractionKind::YouTubeShorts,
                    elapsed: Duration::from_secs(6),
                },
                DistractionEvent::Started(DistractionKind::InstagramReels),
            ]
        );
        assert_eq!(tracker.session.unwrap().started_at, changed_at);
    }

    #[test]
    fn leaving_and_returning_starts_a_fresh_continuous_session() {
        let first_start = Instant::now();
        let mut tracker = active_session(BrowserActivity::YouTubeShorts, first_start);
        tracker.update_desktop_activity(
            ActivityCategory::Terminal,
            first_start + Duration::from_secs(8),
        );
        let second_start = first_start + Duration::from_secs(20);

        assert_eq!(
            tracker.update_desktop_activity(ActivityCategory::Browser, second_start),
            started(DistractionKind::YouTubeShorts)
        );
        assert_eq!(tracker.session.as_ref().unwrap().started_at, second_start);
        assert_eq!(
            tracker.check_thresholds(second_start + Duration::from_secs(10)),
            vec![DistractionEvent::Threshold {
                kind: DistractionKind::YouTubeShorts,
                seconds: FIRST_THRESHOLD_SECONDS,
            }]
        );
    }

    #[test]
    fn thresholds_each_fire_only_once_without_waiting() {
        let started_at = Instant::now();
        let mut tracker = active_session(BrowserActivity::InstagramReels, started_at);

        assert_eq!(
            tracker.check_thresholds(started_at + Duration::from_secs(20)),
            vec![
                DistractionEvent::Threshold {
                    kind: DistractionKind::InstagramReels,
                    seconds: FIRST_THRESHOLD_SECONDS,
                },
                DistractionEvent::Threshold {
                    kind: DistractionKind::InstagramReels,
                    seconds: SECOND_THRESHOLD_SECONDS,
                },
            ]
        );
        assert!(
            tracker
                .check_thresholds(started_at + Duration::from_secs(20))
                .is_empty()
        );
        assert_eq!(
            tracker.check_thresholds(started_at + Duration::from_secs(31)),
            vec![DistractionEvent::Threshold {
                kind: DistractionKind::InstagramReels,
                seconds: THIRD_THRESHOLD_SECONDS,
            }]
        );
        assert!(
            tracker
                .check_thresholds(started_at + Duration::from_secs(60))
                .is_empty()
        );
    }

    #[test]
    fn ending_a_session_reports_any_just_crossed_threshold_first() {
        let started_at = Instant::now();
        let mut tracker = active_session(BrowserActivity::YouTubeShorts, started_at);
        let ended_at = started_at + Duration::from_millis(20_400);

        assert_eq!(
            tracker.update_browser_activity(BrowserActivity::Other, ended_at),
            vec![
                DistractionEvent::Threshold {
                    kind: DistractionKind::YouTubeShorts,
                    seconds: FIRST_THRESHOLD_SECONDS,
                },
                DistractionEvent::Threshold {
                    kind: DistractionKind::YouTubeShorts,
                    seconds: SECOND_THRESHOLD_SECONDS,
                },
                DistractionEvent::Ended {
                    kind: DistractionKind::YouTubeShorts,
                    elapsed: Duration::from_millis(20_400),
                },
            ]
        );
    }

    #[test]
    fn system_idle_ends_session_and_resume_starts_a_new_one() {
        let started_at = Instant::now();
        let mut tracker = active_session(BrowserActivity::YouTubeShorts, started_at);
        let idle_at = started_at + Duration::from_secs(7);

        assert_eq!(
            tracker.update_system_idle(true, idle_at),
            vec![DistractionEvent::Ended {
                kind: DistractionKind::YouTubeShorts,
                elapsed: Duration::from_secs(7),
            }]
        );
        let resumed_at = idle_at + Duration::from_secs(20);
        assert_eq!(
            tracker.update_system_idle(false, resumed_at),
            started(DistractionKind::YouTubeShorts)
        );
        assert_eq!(tracker.session.unwrap().started_at, resumed_at);
    }

    #[test]
    fn browser_disconnect_ends_session() {
        let started_at = Instant::now();
        let mut tracker = active_session(BrowserActivity::InstagramReels, started_at);

        assert_eq!(
            tracker.browser_unavailable(started_at + Duration::from_secs(3)),
            vec![DistractionEvent::Ended {
                kind: DistractionKind::InstagramReels,
                elapsed: Duration::from_secs(3),
            }]
        );
    }

    fn active_session(
        browser_activity: BrowserActivity,
        started_at: Instant,
    ) -> DistractionTracker {
        let mut tracker = DistractionTracker::new();
        tracker.update_desktop_activity(ActivityCategory::Browser, started_at);
        tracker.update_browser_activity(browser_activity, started_at);
        tracker
    }
}
