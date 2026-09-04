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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MiloState {
    #[default]
    Idle,
    Sleeping,
    Curious,
}

impl MiloState {
    pub fn next(self) -> Self {
        match self {
            Self::Idle => Self::Sleeping,
            Self::Sleeping => Self::Curious,
            Self::Curious => Self::Idle,
        }
    }
}

struct AnimationSet {
    frames: Vec<gtk::gdk::Texture>,
    steps: &'static [AnimationStep],
}

struct AnimationSets {
    idle: AnimationSet,
    sleeping: AnimationSet,
    curious: AnimationSet,
}

impl AnimationSets {
    fn load() -> Result<Self, String> {
        Ok(Self {
            idle: load_animation_set(&IDLE_FRAME_PATHS, &IDLE_STEPS)?,
            sleeping: load_animation_set(&SLEEPING_FRAME_PATHS, &SLEEPING_STEPS)?,
            curious: load_animation_set(&CURIOUS_FRAME_PATHS, &CURIOUS_STEPS)?,
        })
    }

    fn get(&self, state: MiloState) -> &AnimationSet {
        match state {
            MiloState::Idle => &self.idle,
            MiloState::Sleeping => &self.sleeping,
            MiloState::Curious => &self.curious,
        }
    }
}

struct PlayerInner {
    picture: gtk::glib::WeakRef<gtk::Picture>,
    animations: AnimationSets,
    state: MiloState,
    step_index: usize,
    timeout: Option<gtk::glib::SourceId>,
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
            })),
        };

        animator.set_state(MiloState::default());
        Ok(animator)
    }

    pub fn state(&self) -> MiloState {
        self.inner.borrow().state
    }

    pub fn set_state(&self, state: MiloState) {
        let duration = {
            let mut player = self.inner.borrow_mut();
            if let Some(timeout) = player.timeout.take() {
                timeout.remove();
            }

            player.state = state;
            player.step_index = 0;

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
    let next_duration = {
        let mut player = inner.borrow_mut();
        player.timeout = None;

        let animation = player.animations.get(player.state);
        player.step_index = (player.step_index + 1) % animation.steps.len();

        let animation = player.animations.get(player.state);
        let step = animation.steps[player.step_index];
        let Some(picture) = player.picture.upgrade() else {
            return;
        };

        picture.set_paintable(Some(&animation.frames[step.frame]));
        Duration::from_millis(step.duration_ms)
    };

    schedule_next_step(inner, next_duration);
}
