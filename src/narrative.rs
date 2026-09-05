use crate::world::{WorldEngine, WorldEvent, WorldObject, WorldProgress};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

pub const DIALOGUE_LINE_DURATION: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NarrativeTrigger {
    FirstLaunch,
    BecameConcerned,
    BreakAccepted,
    ReturnedAfterBreak,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NarrativeMilestone {
    EliRevealed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DialogueLine {
    pub text: &'static str,
    pub duration: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogueSequence {
    lines: Vec<DialogueLine>,
}

impl DialogueSequence {
    pub(crate) fn new(texts: &[&'static str]) -> Self {
        Self {
            lines: texts
                .iter()
                .map(|text| DialogueLine {
                    text,
                    duration: DIALOGUE_LINE_DURATION,
                })
                .collect(),
        }
    }

    pub fn into_lines(self) -> impl Iterator<Item = DialogueLine> {
        self.lines.into_iter()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct NarrativeProgress {
    introduction_seen: bool,
    concerned_dialogue_seen: bool,
    breaks_accepted: u32,
    return_dialogue_seen: bool,
    eli_revealed: bool,
    world: WorldProgress,
}

#[derive(Debug, Eq, PartialEq)]
struct NarrativeUpdate {
    trigger: NarrativeTrigger,
    dialogue: Option<DialogueSequence>,
    milestone: Option<NarrativeMilestone>,
}

#[derive(Debug)]
struct NarrativeEngine {
    progress: NarrativeProgress,
    accepted_break_pending: bool,
    idle_after_accepted_break: bool,
}

impl NarrativeEngine {
    fn new(progress: NarrativeProgress) -> Self {
        Self {
            progress,
            accepted_break_pending: false,
            idle_after_accepted_break: false,
        }
    }

    fn first_launch(&mut self) -> Option<NarrativeUpdate> {
        if self.progress.introduction_seen {
            return None;
        }

        self.progress.introduction_seen = true;
        Some(NarrativeUpdate {
            trigger: NarrativeTrigger::FirstLaunch,
            dialogue: Some(DialogueSequence::new(&["Hi.", "You work here?"])),
            milestone: None,
        })
    }

    fn became_concerned(&mut self) -> Option<NarrativeUpdate> {
        if self.progress.concerned_dialogue_seen {
            return None;
        }

        self.progress.concerned_dialogue_seen = true;
        Some(NarrativeUpdate {
            trigger: NarrativeTrigger::BecameConcerned,
            dialogue: Some(DialogueSequence::new(&[
                "You've been staring at that for a while.",
            ])),
            milestone: None,
        })
    }

    fn break_accepted(&mut self) -> NarrativeUpdate {
        self.progress.breaks_accepted = self.progress.breaks_accepted.saturating_add(1);
        self.accepted_break_pending = true;
        self.idle_after_accepted_break = false;

        let (dialogue, milestone) = match self.progress.breaks_accepted {
            1 => (
                Some(DialogueSequence::new(&[
                    "Good.",
                    "...I mean, I'll be fine.",
                ])),
                None,
            ),
            2 => (
                Some(DialogueSequence::new(&["Going somewhere?", "...Good."])),
                None,
            ),
            3 => {
                let milestone = if self.progress.eli_revealed {
                    None
                } else {
                    self.progress.eli_revealed = true;
                    Some(NarrativeMilestone::EliRevealed)
                };
                (
                    Some(DialogueSequence::new(&[
                        "Found something earlier.",
                        "It has a name on it.",
                        "...Eli.",
                    ])),
                    milestone,
                )
            }
            _ => (None, None),
        };

        NarrativeUpdate {
            trigger: NarrativeTrigger::BreakAccepted,
            dialogue,
            milestone,
        }
    }

    fn system_idle(&mut self) {
        if self.accepted_break_pending {
            self.idle_after_accepted_break = true;
        }
    }

    fn system_resumed(&mut self) -> Option<NarrativeUpdate> {
        if !self.accepted_break_pending || !self.idle_after_accepted_break {
            return None;
        }

        self.accepted_break_pending = false;
        self.idle_after_accepted_break = false;
        if self.progress.return_dialogue_seen {
            return None;
        }

        self.progress.return_dialogue_seen = true;
        Some(NarrativeUpdate {
            trigger: NarrativeTrigger::ReturnedAfterBreak,
            dialogue: Some(DialogueSequence::new(&[
                "You came back.",
                "I wasn't waiting.",
            ])),
            milestone: None,
        })
    }
}

struct NarrativeStore {
    path: Option<PathBuf>,
}

impl NarrativeStore {
    fn load() -> (Self, NarrativeProgress) {
        let path = match narrative_state_path() {
            Ok(path) => Some(path),
            Err(error) => {
                eprintln!("[milo] narrative progress unavailable: {error}");
                None
            }
        };
        let progress = path.as_deref().map(load_progress).unwrap_or_default();
        (Self { path }, progress)
    }

    fn save(&self, progress: &NarrativeProgress) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        if let Err(error) = save_progress(path, progress) {
            eprintln!("[milo] could not save narrative progress: {error}");
        }
    }
}

type DialogueHandler = Rc<RefCell<Box<dyn FnMut(DialogueSequence)>>>;
type WorldEventHandler = Rc<RefCell<Box<dyn FnMut(WorldEvent)>>>;

#[derive(Clone)]
pub struct NarrativeController {
    engine: Rc<RefCell<NarrativeEngine>>,
    store: Rc<NarrativeStore>,
    dialogue_handler: DialogueHandler,
    world: Rc<RefCell<WorldEngine>>,
    world_event_handler: WorldEventHandler,
}

impl NarrativeController {
    pub fn load<F, G>(dialogue_handler: F, world_event_handler: G) -> Self
    where
        F: FnMut(DialogueSequence) + 'static,
        G: FnMut(WorldEvent) + 'static,
    {
        let (store, mut progress) = NarrativeStore::load();
        let mut world = WorldEngine::default();
        let compatibility_event = reconcile_world_progress(&mut world, &mut progress);
        if progress.world.is_visible(WorldObject::EliPhoto) {
            eprintln!("[milo] world object visible: EliPhoto");
        }
        let controller = Self {
            engine: Rc::new(RefCell::new(NarrativeEngine::new(progress))),
            store: Rc::new(store),
            dialogue_handler: Rc::new(RefCell::new(Box::new(dialogue_handler))),
            world: Rc::new(RefCell::new(world)),
            world_event_handler: Rc::new(RefCell::new(Box::new(world_event_handler))),
        };
        if compatibility_event.is_some() {
            controller.commit(None, compatibility_event);
        }
        controller
    }

    pub fn first_launch(&self) {
        let update = self.engine.borrow_mut().first_launch();
        self.commit(update, None);
    }

    pub fn became_concerned(&self) {
        let update = self.engine.borrow_mut().became_concerned();
        self.commit(update, None);
    }

    pub fn break_accepted(&self) {
        let (update, world_event) = {
            let mut engine = self.engine.borrow_mut();
            let update = engine.break_accepted();
            let world_event = apply_world_milestone(
                &mut self.world.borrow_mut(),
                &mut engine.progress,
                update.milestone,
            );
            (update, world_event)
        };
        self.commit(Some(update), world_event);
    }

    pub fn system_idle(&self) {
        let mut engine = self.engine.borrow_mut();
        engine.system_idle();
        self.world.borrow_mut().system_idle(&engine.progress.world);
    }

    pub fn system_resumed(&self) {
        let (update, world_event) = {
            let mut engine = self.engine.borrow_mut();
            let update = engine.system_resumed();
            let world_event = self
                .world
                .borrow_mut()
                .system_resumed(&mut engine.progress.world);
            (update, world_event)
        };
        self.commit(update, world_event);
    }

    pub fn inspect_world_object(&self, object: WorldObject) {
        let world_event = {
            let mut engine = self.engine.borrow_mut();
            self.world
                .borrow_mut()
                .inspect(&mut engine.progress.world, object)
        };
        self.commit(None, world_event);
    }

    pub fn is_world_object_visible(&self, object: WorldObject) -> bool {
        self.engine.borrow().progress.world.is_visible(object)
    }

    fn commit(&self, update: Option<NarrativeUpdate>, world_event: Option<WorldEvent>) {
        if update.is_none() && world_event.is_none() {
            return;
        }

        self.store.save(&self.engine.borrow().progress);

        if let Some(update) = update {
            self.handle_narrative_update(update);
        }
        if let Some(world_event) = world_event {
            self.handle_world_event(world_event);
        }
    }

    fn handle_narrative_update(&self, update: NarrativeUpdate) {
        eprintln!("[milo] narrative trigger: {:?}", update.trigger);
        if update.trigger == NarrativeTrigger::BreakAccepted {
            eprintln!(
                "[milo] narrative progress: breaks_accepted={}",
                self.engine.borrow().progress.breaks_accepted
            );
        }
        if let Some(milestone) = update.milestone {
            eprintln!("[milo] narrative milestone: {milestone:?}");
        }
        if let Some(dialogue) = update.dialogue {
            (self.dialogue_handler.borrow_mut())(dialogue);
        }
    }

    fn handle_world_event(&self, event: WorldEvent) {
        match event {
            WorldEvent::ObjectPending(WorldObject::EliPhoto) => {
                eprintln!("[milo] world object pending: EliPhoto");
            }
            WorldEvent::EliPhotoAppeared => {
                eprintln!("[milo] world event: EliPhotoAppeared");
                eprintln!("[milo] world object visible: EliPhoto");
            }
            WorldEvent::ObjectInspected(WorldObject::EliPhoto) => {
                eprintln!("[milo] world object inspected: EliPhoto");
            }
        }
        (self.world_event_handler.borrow_mut())(event);
    }
}

fn apply_world_milestone(
    world: &mut WorldEngine,
    progress: &mut NarrativeProgress,
    milestone: Option<NarrativeMilestone>,
) -> Option<WorldEvent> {
    match milestone {
        Some(NarrativeMilestone::EliRevealed) => world.eli_revealed(&mut progress.world),
        None => None,
    }
}

fn reconcile_world_progress(
    world: &mut WorldEngine,
    progress: &mut NarrativeProgress,
) -> Option<WorldEvent> {
    if progress.eli_revealed {
        world.eli_revealed(&mut progress.world)
    } else {
        None
    }
}

fn narrative_state_path() -> io::Result<PathBuf> {
    if let Some(state_home) = env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(state_home)
            .join("bloomaway")
            .join("narrative.json"));
    }

    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is missing"))?;
    Ok(PathBuf::from(home)
        .join(".local/state/bloomaway")
        .join("narrative.json"))
}

fn load_progress(path: &Path) -> NarrativeProgress {
    match fs::read(path) {
        Ok(payload) => decode_progress_or_default(&payload),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!("[milo] narrative progress not found; starting fresh");
            NarrativeProgress::default()
        }
        Err(error) => {
            eprintln!("[milo] could not load narrative progress: {error}");
            NarrativeProgress::default()
        }
    }
}

fn decode_progress_or_default(payload: &[u8]) -> NarrativeProgress {
    match serde_json::from_slice(payload) {
        Ok(progress) => progress,
        Err(error) => {
            eprintln!("[milo] malformed narrative progress; starting fresh: {error}");
            NarrativeProgress::default()
        }
    }
}

fn save_progress(path: &Path, progress: &NarrativeProgress) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "narrative path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;

    let temporary_path = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temporary_path)?;
        serde_json::to_writer_pretty(&mut file, progress).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialogue_text(update: &NarrativeUpdate) -> Vec<&'static str> {
        update
            .dialogue
            .clone()
            .into_iter()
            .flat_map(DialogueSequence::into_lines)
            .map(|line| line.text)
            .collect()
    }

    #[test]
    fn fresh_state_produces_first_launch_dialogue_once() {
        let mut engine = NarrativeEngine::new(NarrativeProgress::default());

        let update = engine.first_launch().unwrap();
        assert_eq!(update.trigger, NarrativeTrigger::FirstLaunch);
        assert_eq!(dialogue_text(&update), ["Hi.", "You work here?"]);
        assert!(engine.first_launch().is_none());
    }

    #[test]
    fn first_real_concerned_event_produces_dialogue_once() {
        let mut engine = NarrativeEngine::new(NarrativeProgress::default());

        let update = engine.became_concerned().unwrap();
        assert_eq!(update.trigger, NarrativeTrigger::BecameConcerned);
        assert_eq!(
            dialogue_text(&update),
            ["You've been staring at that for a while."]
        );
        assert!(engine.became_concerned().is_none());
    }

    #[test]
    fn unrelated_state_changes_do_not_trigger_concerned_dialogue() {
        let mut engine = NarrativeEngine::new(NarrativeProgress::default());

        engine.system_idle();
        assert!(engine.system_resumed().is_none());
        assert!(!engine.progress.concerned_dialogue_seen);
    }

    #[test]
    fn first_and_second_breaks_return_their_dialogue() {
        let mut engine = NarrativeEngine::new(NarrativeProgress::default());

        let first = engine.break_accepted();
        assert_eq!(dialogue_text(&first), ["Good.", "...I mean, I'll be fine."]);
        assert_eq!(engine.progress.breaks_accepted, 1);

        let second = engine.break_accepted();
        assert_eq!(dialogue_text(&second), ["Going somewhere?", "...Good."]);
        assert_eq!(engine.progress.breaks_accepted, 2);
    }

    #[test]
    fn third_break_reveals_eli_and_fourth_does_not_repeat_it() {
        let mut engine = NarrativeEngine::new(NarrativeProgress::default());
        let mut world = WorldEngine::default();
        engine.break_accepted();
        engine.break_accepted();

        let third = engine.break_accepted();
        assert_eq!(
            dialogue_text(&third),
            [
                "Found something earlier.",
                "It has a name on it.",
                "...Eli."
            ]
        );
        assert_eq!(third.milestone, Some(NarrativeMilestone::EliRevealed));
        assert!(engine.progress.eli_revealed);
        assert_eq!(
            apply_world_milestone(&mut world, &mut engine.progress, third.milestone),
            Some(WorldEvent::ObjectPending(WorldObject::EliPhoto))
        );
        assert!(!engine.progress.world.is_visible(WorldObject::EliPhoto));

        let fourth = engine.break_accepted();
        assert!(fourth.dialogue.is_none());
        assert_eq!(fourth.milestone, None);
        assert_eq!(engine.progress.breaks_accepted, 4);
    }

    #[test]
    fn return_dialogue_requires_break_then_idle_then_resume() {
        let mut engine = NarrativeEngine::new(NarrativeProgress::default());

        assert!(engine.system_resumed().is_none());
        engine.break_accepted();
        assert!(engine.system_resumed().is_none());
        engine.system_idle();
        let returned = engine.system_resumed().unwrap();

        assert_eq!(returned.trigger, NarrativeTrigger::ReturnedAfterBreak);
        assert_eq!(
            dialogue_text(&returned),
            ["You came back.", "I wasn't waiting."]
        );
        assert!(engine.system_resumed().is_none());
    }

    #[test]
    fn ordinary_idle_and_resume_has_no_return_dialogue() {
        let mut engine = NarrativeEngine::new(NarrativeProgress::default());

        engine.system_idle();
        assert!(engine.system_resumed().is_none());
        assert!(!engine.progress.return_dialogue_seen);
    }

    #[test]
    fn progress_round_trips_through_json() {
        let expected = NarrativeProgress {
            introduction_seen: true,
            concerned_dialogue_seen: true,
            breaks_accepted: 3,
            return_dialogue_seen: true,
            eli_revealed: true,
            world: WorldProgress::default(),
        };

        let encoded = serde_json::to_vec(&expected).unwrap();
        let decoded: NarrativeProgress = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn chapter_one_progress_without_world_data_remains_compatible() {
        let decoded = decode_progress_or_default(
            br#"{
                "introduction_seen": true,
                "concerned_dialogue_seen": true,
                "breaks_accepted": 3,
                "return_dialogue_seen": true,
                "eli_revealed": true
            }"#,
        );

        assert!(decoded.introduction_seen);
        assert!(decoded.concerned_dialogue_seen);
        assert_eq!(decoded.breaks_accepted, 3);
        assert!(decoded.return_dialogue_seen);
        assert!(decoded.eli_revealed);
        assert!(!decoded.world.is_visible(WorldObject::EliPhoto));

        let mut world = WorldEngine::default();
        let mut reconciled = decoded.clone();
        assert_eq!(
            reconcile_world_progress(&mut world, &mut reconciled),
            Some(WorldEvent::ObjectPending(WorldObject::EliPhoto))
        );
        assert!(!reconciled.world.is_visible(WorldObject::EliPhoto));
    }

    #[test]
    fn malformed_progress_safely_falls_back() {
        assert_eq!(
            decode_progress_or_default(br#"{"breaks_accepted": "many"}"#),
            NarrativeProgress::default()
        );
    }
}
