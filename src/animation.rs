use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

const IDLE_FRAME_PATHS: [&str; 4] = [
    "assets/milo/idle/idle_01.png",
    "assets/milo/idle/idle_02.png",
    "assets/milo/idle/idle_03.png",
    "assets/milo/idle/idle_04.png",
];

const SLEEPING_FRAME_PATHS: [&str; 4] = [
    "assets/milo/sleeping/sleeping_01.png",
    "assets/milo/sleeping/sleeping_02.png",
    "assets/milo/sleeping/sleeping_03.png",
    "assets/milo/sleeping/sleeping_04.png",
];

const CURIOUS_FRAME_PATHS: [&str; 4] = [
    "assets/milo/curious/curious_01.png",
    "assets/milo/curious/curious_02.png",
    "assets/milo/curious/curious_03.png",
    "assets/milo/curious/curious_04.png",
];

const CONCERNED_FRAME_PATHS: [&str; 4] = [
    "assets/milo/concerned/concerned_01.png",
    "assets/milo/concerned/concerned_02.png",
    "assets/milo/concerned/concerned_03.png",
    "assets/milo/concerned/concerned_04.png",
];

const PLAY_WITH_YARN_FRAME_PATHS: [&str; 8] = [
    "assets/milo/play_yarn/play_yarn_01.png",
    "assets/milo/play_yarn/play_yarn_02.png",
    "assets/milo/play_yarn/play_yarn_03.png",
    "assets/milo/play_yarn/play_yarn_04.png",
    "assets/milo/play_yarn/play_yarn_05.png",
    "assets/milo/play_yarn/play_yarn_06.png",
    "assets/milo/play_yarn/play_yarn_07.png",
    "assets/milo/play_yarn/play_yarn_08.png",
];

#[derive(Clone, Copy)]
struct AnimationStep {
    frame: usize,
    duration_ms: u64,
}

const IDLE_STEPS: [AnimationStep; 5] = [
    AnimationStep {
        frame: 0,
        duration_ms: 600,
    },
    AnimationStep {
        frame: 1,
        duration_ms: 350,
    },
    AnimationStep {
        frame: 2,
        duration_ms: 150,
    },
    AnimationStep {
        frame: 3,
        duration_ms: 350,
    },
    AnimationStep {
        frame: 0,
        duration_ms: 900,
    },
];

const SLEEPING_STEPS: [AnimationStep; 4] = [
    AnimationStep {
        frame: 0,
        duration_ms: 700,
    },
    AnimationStep {
        frame: 1,
        duration_ms: 450,
    },
    AnimationStep {
        frame: 2,
        duration_ms: 450,
    },
    AnimationStep {
        frame: 3,
        duration_ms: 700,
    },
];

const CURIOUS_STEPS: [AnimationStep; 4] = [
    AnimationStep {
        frame: 0,
        duration_ms: 350,
    },
    AnimationStep {
        frame: 1,
        duration_ms: 250,
    },
    AnimationStep {
        frame: 2,
        duration_ms: 300,
    },
    AnimationStep {
        frame: 3,
        duration_ms: 350,
    },
];

const CONCERNED_STEPS: [AnimationStep; 4] = [
    AnimationStep {
        frame: 0,
        duration_ms: 500,
    },
    AnimationStep {
        frame: 1,
        duration_ms: 400,
    },
    AnimationStep {
        frame: 2,
        duration_ms: 650,
    },
    AnimationStep {
        frame: 3,
        duration_ms: 400,
    },
];

const PLAY_WITH_YARN_STEPS: [AnimationStep; 8] = [
    AnimationStep {
        frame: 0,
        duration_ms: 180,
    },
    AnimationStep {
        frame: 1,
        duration_ms: 180,
    },
    AnimationStep {
        frame: 2,
        duration_ms: 180,
    },
    AnimationStep {
        frame: 3,
        duration_ms: 220,
    },
    AnimationStep {
        frame: 4,
        duration_ms: 180,
    },
    AnimationStep {
        frame: 5,
        duration_ms: 220,
    },
    AnimationStep {
        frame: 6,
        duration_ms: 220,
    },
    AnimationStep {
        frame: 7,
        duration_ms: 260,
    },
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MiloState {
    #[default]
    Idle,
    Sleeping,
    Curious,
    Concerned,
    PlayWithYarn,
}

impl MiloState {
    pub fn next(self) -> Self {
        match self {
            Self::Idle => Self::Sleeping,
            Self::Sleeping => Self::Curious,
            Self::Curious => Self::Concerned,
            Self::Concerned => Self::PlayWithYarn,
            Self::PlayWithYarn => Self::Idle,
        }
    }
}

struct AnimationSet {
    frames: Vec<gtk::gdk::Texture>,
    steps: &'static [AnimationStep],
}

struct LoopCountdown {
    remaining: usize,
}

impl LoopCountdown {
    fn new(loops: usize) -> Self {
        assert!(
            loops > 0,
            "finite animation playback needs at least one loop"
        );
        Self { remaining: loops }
    }

    fn completed_loop(&mut self) -> bool {
        self.remaining -= 1;
        self.remaining == 0
    }
}

struct FinitePlayback {
    countdown: LoopCountdown,
    on_complete: Box<dyn FnOnce()>,
}

struct AnimationSets {
    idle: AnimationSet,
    sleeping: AnimationSet,
    curious: AnimationSet,
    concerned: AnimationSet,
    play_with_yarn: AnimationSet,
}

impl AnimationSets {
    fn load() -> Result<Self, String> {
        Ok(Self {
            idle: load_animation_set(&IDLE_FRAME_PATHS, &IDLE_STEPS)?,
            sleeping: load_animation_set(&SLEEPING_FRAME_PATHS, &SLEEPING_STEPS)?,
            curious: load_animation_set(&CURIOUS_FRAME_PATHS, &CURIOUS_STEPS)?,
            concerned: load_animation_set(&CONCERNED_FRAME_PATHS, &CONCERNED_STEPS)?,
            play_with_yarn: load_animation_set(&PLAY_WITH_YARN_FRAME_PATHS, &PLAY_WITH_YARN_STEPS)?,
        })
    }

    fn get(&self, state: MiloState) -> &AnimationSet {
        match state {
            MiloState::Idle => &self.idle,
            MiloState::Sleeping => &self.sleeping,
            MiloState::Curious => &self.curious,
            MiloState::Concerned => &self.concerned,
            MiloState::PlayWithYarn => &self.play_with_yarn,
        }
    }
}

struct PlayerInner {
    picture: gtk::glib::WeakRef<gtk::Picture>,
    animations: AnimationSets,
    state: MiloState,
    step_index: usize,
    timeout: Option<gtk::glib::SourceId>,
    finite_playback: Option<FinitePlayback>,
}

#[derive(Clone)]
pub struct MiloAnimator {
    inner: Rc<RefCell<PlayerInner>>,
}

impl MiloAnimator {
    pub fn new(picture: &gtk::Picture) -> Result<Self, String> {
        let animator = Self {
            inner: Rc::new(RefCell::new(PlayerInner {
                picture: picture.downgrade(),
                animations: AnimationSets::load()?,
                state: MiloState::default(),
                step_index: 0,
                timeout: None,
                finite_playback: None,
            })),
        };

        animator.set_state(MiloState::default());
        Ok(animator)
    }

    pub fn set_state(&self, state: MiloState) {
        self.start_animation(state, None);
    }

    pub fn play_loops<F>(&self, state: MiloState, loops: usize, on_complete: F)
    where
        F: FnOnce() + 'static,
    {
        self.start_animation(
            state,
            Some(FinitePlayback {
                countdown: LoopCountdown::new(loops),
                on_complete: Box::new(on_complete),
            }),
        );
    }

    fn start_animation(&self, state: MiloState, finite_playback: Option<FinitePlayback>) {
        let duration = {
            let mut player = self.inner.borrow_mut();
            if let Some(timeout) = player.timeout.take() {
                timeout.remove();
            }

            player.state = state;
            player.step_index = 0;
            player.finite_playback = finite_playback;

            let animation = player.animations.get(state);
            let step = animation.steps[0];
            if let Some(picture) = player.picture.upgrade() {
                picture.set_paintable(Some(&animation.frames[step.frame]));
            }

            Duration::from_millis(step.duration_ms)
        };

        schedule_next_step(Rc::clone(&self.inner), duration);
    }
}

fn load_animation_set(
    frame_paths: &[&str],
    steps: &'static [AnimationStep],
) -> Result<AnimationSet, String> {
    let frames = frame_paths
        .iter()
        .map(|relative_path| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
            gtk::gdk::Texture::from_filename(&path)
                .map_err(|error| format!("failed to load Milo frame {}: {error}", path.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(AnimationSet { frames, steps })
}

fn schedule_next_step(inner: Rc<RefCell<PlayerInner>>, duration: Duration) {
    let callback_inner = Rc::clone(&inner);
    let timeout = gtk::glib::timeout_add_local_once(duration, move || {
        advance_frame(callback_inner);
    });
    inner.borrow_mut().timeout = Some(timeout);
}

fn advance_frame(inner: Rc<RefCell<PlayerInner>>) {
    let (next_duration, completion) = {
        let mut player = inner.borrow_mut();
        player.timeout = None;

        let step_count = player.animations.get(player.state).steps.len();
        let next_step_index = (player.step_index + 1) % step_count;
        let playback_complete = next_step_index == 0
            && player
                .finite_playback
                .as_mut()
                .is_some_and(|playback| playback.countdown.completed_loop());

        if playback_complete {
            let completion = player
                .finite_playback
                .take()
                .map(|playback| playback.on_complete);
            (None, completion)
        } else {
            player.step_index = next_step_index;

            let animation = player.animations.get(player.state);
            let step = animation.steps[player.step_index];
            let Some(picture) = player.picture.upgrade() else {
                return;
            };

            picture.set_paintable(Some(&animation.frames[step.frame]));
            (Some(Duration::from_millis(step.duration_ms)), None)
        }
    };

    if let Some(completion) = completion {
        completion();
    } else if let Some(next_duration) = next_duration {
        schedule_next_step(inner, next_duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_cycle_includes_every_animation_state() {
        assert_eq!(MiloState::Idle.next(), MiloState::Sleeping);
        assert_eq!(MiloState::Sleeping.next(), MiloState::Curious);
        assert_eq!(MiloState::Curious.next(), MiloState::Concerned);
        assert_eq!(MiloState::Concerned.next(), MiloState::PlayWithYarn);
        assert_eq!(MiloState::PlayWithYarn.next(), MiloState::Idle);
    }

    #[test]
    fn play_with_yarn_uses_all_frames_with_the_requested_timing() {
        assert_eq!(
            PLAY_WITH_YARN_STEPS.map(|step| step.frame),
            [0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(
            PLAY_WITH_YARN_STEPS.map(|step| step.duration_ms),
            [180, 180, 180, 220, 180, 220, 220, 260]
        );
    }

    #[test]
    fn finite_playback_completes_after_exactly_three_loops() {
        let mut countdown = LoopCountdown::new(3);

        assert!(!countdown.completed_loop());
        assert!(!countdown.completed_loop());
        assert!(countdown.completed_loop());
    }
}
