use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldObject {
    EliPhoto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorldEvent {
    ObjectPending(WorldObject),
    EliPhotoAppeared,
    ObjectInspected(WorldObject),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WorldProgress {
    eli_photo_pending: bool,
    eli_photo_visible: bool,
    eli_photo_inspected: bool,
    eli_photo_appearance_dialogue_seen: bool,
}

impl WorldProgress {
    pub fn is_visible(&self, object: WorldObject) -> bool {
        match object {
            WorldObject::EliPhoto => self.eli_photo_visible,
        }
    }
}

#[derive(Debug, Default)]
pub struct WorldEngine {
    eli_photo_away_observed: bool,
}

impl WorldEngine {
    pub fn eli_revealed(&mut self, progress: &mut WorldProgress) -> Option<WorldEvent> {
        if progress.eli_photo_pending || progress.eli_photo_visible {
            return None;
        }

        progress.eli_photo_pending = true;
        Some(WorldEvent::ObjectPending(WorldObject::EliPhoto))
    }

    pub fn system_idle(&mut self, progress: &WorldProgress) {
        if progress.eli_photo_pending && !progress.eli_photo_visible {
            self.eli_photo_away_observed = true;
        }
    }

    pub fn system_resumed(&mut self, progress: &mut WorldProgress) -> Option<WorldEvent> {
        if !self.eli_photo_away_observed {
            return None;
        }
        self.eli_photo_away_observed = false;

        if !progress.eli_photo_pending || progress.eli_photo_visible {
            return None;
        }

        progress.eli_photo_pending = false;
        progress.eli_photo_visible = true;
        progress.eli_photo_appearance_dialogue_seen = true;
        Some(WorldEvent::EliPhotoAppeared)
    }

    pub fn inspect(
        &mut self,
        progress: &mut WorldProgress,
        object: WorldObject,
    ) -> Option<WorldEvent> {
        match object {
            WorldObject::EliPhoto
                if progress.eli_photo_visible && !progress.eli_photo_inspected =>
            {
                progress.eli_photo_inspected = true;
                Some(WorldEvent::ObjectInspected(WorldObject::EliPhoto))
            }
            WorldObject::EliPhoto => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible_photo() -> (WorldEngine, WorldProgress) {
        let mut engine = WorldEngine::default();
        let mut progress = WorldProgress::default();
        engine.eli_revealed(&mut progress);
        engine.system_idle(&progress);
        engine.system_resumed(&mut progress);
        (engine, progress)
    }

    #[test]
    fn fresh_world_has_no_photo() {
        let progress = WorldProgress::default();

        assert!(!progress.eli_photo_pending);
        assert!(!progress.is_visible(WorldObject::EliPhoto));
        assert!(!progress.eli_photo_inspected);
    }

    #[test]
    fn eli_revelation_marks_photo_pending_without_showing_it() {
        let mut engine = WorldEngine::default();
        let mut progress = WorldProgress::default();

        assert_eq!(
            engine.eli_revealed(&mut progress),
            Some(WorldEvent::ObjectPending(WorldObject::EliPhoto))
        );
        assert!(progress.eli_photo_pending);
        assert!(!progress.is_visible(WorldObject::EliPhoto));
    }

    #[test]
    fn ordinary_resume_does_not_reveal_pending_photo() {
        let mut engine = WorldEngine::default();
        let mut progress = WorldProgress::default();
        engine.eli_revealed(&mut progress);

        assert_eq!(engine.system_resumed(&mut progress), None);
        assert!(!progress.is_visible(WorldObject::EliPhoto));
    }

    #[test]
    fn idle_then_resume_reveals_pending_photo_once() {
        let mut engine = WorldEngine::default();
        let mut progress = WorldProgress::default();
        engine.eli_revealed(&mut progress);

        engine.system_idle(&progress);
        assert_eq!(
            engine.system_resumed(&mut progress),
            Some(WorldEvent::EliPhotoAppeared)
        );
        assert!(progress.is_visible(WorldObject::EliPhoto));
        assert!(!progress.eli_photo_pending);
        assert!(progress.eli_photo_appearance_dialogue_seen);

        engine.system_idle(&progress);
        assert_eq!(engine.system_resumed(&mut progress), None);
    }

    #[test]
    fn visible_photo_survives_serialization() {
        let (_, progress) = visible_photo();
        let encoded = serde_json::to_vec(&progress).unwrap();
        let decoded: WorldProgress = serde_json::from_slice(&encoded).unwrap();

        assert!(decoded.is_visible(WorldObject::EliPhoto));
        assert!(decoded.eli_photo_appearance_dialogue_seen);
    }

    #[test]
    fn pending_photo_survives_serialization_without_becoming_visible() {
        let mut engine = WorldEngine::default();
        let mut progress = WorldProgress::default();
        engine.eli_revealed(&mut progress);

        let encoded = serde_json::to_vec(&progress).unwrap();
        let decoded: WorldProgress = serde_json::from_slice(&encoded).unwrap();

        assert!(decoded.eli_photo_pending);
        assert!(!decoded.is_visible(WorldObject::EliPhoto));
    }

    #[test]
    fn first_inspection_is_persisted_and_cannot_repeat() {
        let (mut engine, mut progress) = visible_photo();

        assert_eq!(
            engine.inspect(&mut progress, WorldObject::EliPhoto),
            Some(WorldEvent::ObjectInspected(WorldObject::EliPhoto))
        );
        assert!(progress.eli_photo_inspected);
        assert_eq!(engine.inspect(&mut progress, WorldObject::EliPhoto), None);
    }

    #[test]
    fn inspection_survives_restart_and_does_not_replay() {
        let (mut engine, mut progress) = visible_photo();
        engine.inspect(&mut progress, WorldObject::EliPhoto);

        let encoded = serde_json::to_vec(&progress).unwrap();
        let mut decoded: WorldProgress = serde_json::from_slice(&encoded).unwrap();
        let mut restarted_engine = WorldEngine::default();

        assert!(decoded.eli_photo_inspected);
        assert_eq!(
            restarted_engine.inspect(&mut decoded, WorldObject::EliPhoto),
            None
        );
    }

    #[test]
    fn hidden_photo_cannot_be_inspected() {
        let mut engine = WorldEngine::default();
        let mut progress = WorldProgress::default();

        assert_eq!(engine.inspect(&mut progress, WorldObject::EliPhoto), None);
        assert!(!progress.eli_photo_inspected);
    }
}
