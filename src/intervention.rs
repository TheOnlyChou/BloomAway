use crate::distraction::{DistractionEvent, THIRD_THRESHOLD_SECONDS};
use std::cell::RefCell;
use std::rc::Rc;

pub const STILL_SCROLLING_PROMPT: &str = "Still scrolling?";
pub const TAKE_BREAK_LABEL: &str = "Take a break";
pub const KEEP_SCROLLING_LABEL: &str = "Keep scrolling";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Intervention {
    StillScrolling,
}

impl Intervention {
    pub fn prompt(self) -> &'static str {
        match self {
            Self::StillScrolling => STILL_SCROLLING_PROMPT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterventionResponse {
    TakeBreak,
    KeepScrolling,
}

impl InterventionResponse {
    pub fn label(self) -> &'static str {
        match self {
            Self::TakeBreak => TAKE_BREAK_LABEL,
            Self::KeepScrolling => KEEP_SCROLLING_LABEL,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterventionPresentation {
    Show(Intervention),
    Hide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterventionAction {
    Present(InterventionPresentation),
    Response(InterventionResponse),
}

#[derive(Debug)]
struct InterventionLifecycle {
    session_active: bool,
    requested_this_session: bool,
    visible: Option<Intervention>,
}

impl InterventionLifecycle {
    fn new() -> Self {
        Self {
            session_active: false,
            requested_this_session: false,
            visible: None,
        }
    }

    fn handle_distraction_event(&mut self, event: DistractionEvent) -> Vec<InterventionAction> {
        match event {
            DistractionEvent::Started(_) => {
                let actions = self.hide_if_visible();
                self.session_active = true;
                self.requested_this_session = false;
                actions
            }
            DistractionEvent::Threshold { seconds, .. }
                if seconds == THIRD_THRESHOLD_SECONDS
                    && self.session_active
                    && !self.requested_this_session =>
            {
                let intervention = Intervention::StillScrolling;
                self.requested_this_session = true;
                self.visible = Some(intervention);
                vec![InterventionAction::Present(InterventionPresentation::Show(
                    intervention,
                ))]
            }
            DistractionEvent::Threshold { .. } => Vec::new(),
            DistractionEvent::Ended { .. } => self.end_session(),
        }
    }

    fn respond(&mut self, response: InterventionResponse) -> Vec<InterventionAction> {
        if self.visible.take().is_none() {
            return Vec::new();
        }

        vec![
            InterventionAction::Response(response),
            InterventionAction::Present(InterventionPresentation::Hide),
        ]
    }

    fn system_idle(&mut self) -> Vec<InterventionAction> {
        self.end_session()
    }

    fn end_session(&mut self) -> Vec<InterventionAction> {
        self.session_active = false;
        self.requested_this_session = false;
        self.hide_if_visible()
    }

    fn hide_if_visible(&mut self) -> Vec<InterventionAction> {
        if self.visible.take().is_some() {
            vec![InterventionAction::Present(InterventionPresentation::Hide)]
        } else {
            Vec::new()
        }
    }
}

type PresentationHandler = Rc<RefCell<Box<dyn FnMut(InterventionPresentation)>>>;
type ResponseHandler = Rc<RefCell<Box<dyn FnMut(InterventionResponse)>>>;

#[derive(Clone)]
pub struct InterventionController {
    lifecycle: Rc<RefCell<InterventionLifecycle>>,
    presentation_handler: PresentationHandler,
    response_handler: ResponseHandler,
}

impl InterventionController {
    pub fn new<F, G>(presentation_handler: F, response_handler: G) -> Self
    where
        F: FnMut(InterventionPresentation) + 'static,
        G: FnMut(InterventionResponse) + 'static,
    {
        Self {
            lifecycle: Rc::new(RefCell::new(InterventionLifecycle::new())),
            presentation_handler: Rc::new(RefCell::new(Box::new(presentation_handler))),
            response_handler: Rc::new(RefCell::new(Box::new(response_handler))),
        }
    }

    pub fn handle_distraction_event(&self, event: DistractionEvent) {
        let actions = self.lifecycle.borrow_mut().handle_distraction_event(event);
        self.handle_actions(actions);
    }

    pub fn respond(&self, response: InterventionResponse) {
        let actions = self.lifecycle.borrow_mut().respond(response);
        self.handle_actions(actions);
    }

    pub fn system_idle(&self) {
        let actions = self.lifecycle.borrow_mut().system_idle();
        self.handle_actions(actions);
    }

    fn handle_actions(&self, actions: Vec<InterventionAction>) {
        for action in actions {
            match action {
                InterventionAction::Present(presentation) => {
                    match presentation {
                        InterventionPresentation::Show(intervention) => {
                            eprintln!("[milo] intervention requested: {intervention:?}");
                            eprintln!("[milo] intervention presentation: Show");
                        }
                        InterventionPresentation::Hide => {
                            eprintln!("[milo] intervention presentation: Hide");
                        }
                    }
                    (self.presentation_handler.borrow_mut())(presentation);
                }
                InterventionAction::Response(response) => {
                    eprintln!("[milo] intervention response: {response:?}");
                    (self.response_handler.borrow_mut())(response);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distraction::DistractionKind;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    fn started(kind: DistractionKind) -> DistractionEvent {
        DistractionEvent::Started(kind)
    }

    fn threshold(seconds: u64) -> DistractionEvent {
        DistractionEvent::Threshold {
            kind: DistractionKind::YouTubeShorts,
            seconds,
        }
    }

    fn ended(kind: DistractionKind) -> DistractionEvent {
        DistractionEvent::Ended {
            kind,
            elapsed: Duration::from_secs(35),
        }
    }

    fn show_still_scrolling() -> Vec<InterventionAction> {
        vec![InterventionAction::Present(InterventionPresentation::Show(
            Intervention::StillScrolling,
        ))]
    }

    fn active_lifecycle() -> InterventionLifecycle {
        let mut lifecycle = InterventionLifecycle::new();
        lifecycle.handle_distraction_event(started(DistractionKind::YouTubeShorts));
        lifecycle
    }

    #[test]
    fn thirty_second_threshold_requests_still_scrolling_once() {
        let mut lifecycle = active_lifecycle();

        assert_eq!(
            lifecycle.handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS)),
            show_still_scrolling()
        );
        assert!(
            lifecycle
                .handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS))
                .is_empty()
        );
    }

    #[test]
    fn controller_delivers_show_to_the_presentation_callback() {
        let presentations = Rc::new(RefCell::new(Vec::new()));
        let captured_presentations = Rc::clone(&presentations);
        let controller = InterventionController::new(
            move |presentation| {
                captured_presentations.borrow_mut().push(presentation);
            },
            |_| {},
        );

        controller.handle_distraction_event(started(DistractionKind::YouTubeShorts));
        controller.handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS));

        assert_eq!(
            *presentations.borrow(),
            vec![InterventionPresentation::Show(Intervention::StillScrolling)]
        );
    }

    #[test]
    fn earlier_thresholds_do_not_request_an_intervention() {
        let mut lifecycle = active_lifecycle();

        assert!(
            lifecycle
                .handle_distraction_event(threshold(crate::distraction::FIRST_THRESHOLD_SECONDS))
                .is_empty()
        );
        assert!(
            lifecycle
                .handle_distraction_event(threshold(crate::distraction::SECOND_THRESHOLD_SECONDS))
                .is_empty()
        );
        assert_eq!(lifecycle.visible, None);
    }

    #[test]
    fn responses_dismiss_without_ending_the_session_or_resetting_eligibility() {
        for response in [
            InterventionResponse::TakeBreak,
            InterventionResponse::KeepScrolling,
        ] {
            let mut lifecycle = active_lifecycle();
            lifecycle.handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS));

            assert_eq!(
                lifecycle.respond(response),
                vec![
                    InterventionAction::Response(response),
                    InterventionAction::Present(InterventionPresentation::Hide),
                ]
            );
            assert!(lifecycle.session_active);
            assert!(lifecycle.requested_this_session);
            assert_eq!(lifecycle.visible, None);
            assert!(
                lifecycle
                    .handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS))
                    .is_empty()
            );
        }
    }

    #[test]
    fn session_end_dismisses_and_resets_eligibility() {
        let mut lifecycle = active_lifecycle();
        lifecycle.handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS));

        assert_eq!(
            lifecycle.handle_distraction_event(ended(DistractionKind::YouTubeShorts)),
            vec![InterventionAction::Present(InterventionPresentation::Hide)]
        );
        assert!(!lifecycle.session_active);
        assert!(!lifecycle.requested_this_session);
    }

    #[test]
    fn system_idle_dismisses_and_ends_intervention_session() {
        let mut lifecycle = active_lifecycle();
        lifecycle.handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS));

        assert_eq!(
            lifecycle.system_idle(),
            vec![InterventionAction::Present(InterventionPresentation::Hide)]
        );
        assert!(!lifecycle.session_active);
        assert!(!lifecycle.requested_this_session);
    }

    #[test]
    fn kind_switch_dismisses_and_new_session_can_request_again() {
        let mut lifecycle = active_lifecycle();
        lifecycle.handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS));

        assert_eq!(
            lifecycle.handle_distraction_event(ended(DistractionKind::YouTubeShorts)),
            vec![InterventionAction::Present(InterventionPresentation::Hide)]
        );
        assert!(
            lifecycle
                .handle_distraction_event(started(DistractionKind::InstagramReels))
                .is_empty()
        );
        assert_eq!(
            lifecycle.handle_distraction_event(threshold(THIRD_THRESHOLD_SECONDS)),
            show_still_scrolling()
        );
    }
}
