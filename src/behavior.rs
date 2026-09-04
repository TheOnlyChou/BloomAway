use crate::animation::{MiloAnimator, MiloState};
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

pub const IDLE_TIMEOUT_SECONDS: u32 = 10;
pub const CURIOUS_AFTER_RESUME_MS: u64 = 3_000;

#[derive(Clone, Copy, Debug)]
enum ActivityEvent {
    Idled,
    Resumed,
}

struct BehaviorInner {
    animator: MiloAnimator,
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
                curious_timeout: RefCell::new(None),
            }),
        }
    }

    pub fn start_idle_monitor(&self) {
        let (sender, mut receiver) = unbounded();
        let controller = self.clone();

        glib::MainContext::default().spawn_local(async move {
            while let Some(event) = receiver.next().await {
                controller.handle_activity_event(event);
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
        self.cancel_curious_timeout();
        let next_state = self.inner.animator.state().next();
        self.inner.animator.set_state(next_state);
        next_state
    }

    fn handle_activity_event(&self, event: ActivityEvent) {
        self.cancel_curious_timeout();

        match event {
            ActivityEvent::Idled => {
                self.inner.animator.set_state(MiloState::Sleeping);
                eprintln!("Milo automatic state: Sleeping (user idle)");
            }
            ActivityEvent::Resumed => {
                self.inner.animator.set_state(MiloState::Curious);
                eprintln!("Milo automatic state: Curious (user resumed)");
                self.schedule_curious_return();
            }
        }
    }

    fn schedule_curious_return(&self) {
        let weak_inner = Rc::downgrade(&self.inner);
        let timeout = glib::timeout_add_local_once(
            Duration::from_millis(CURIOUS_AFTER_RESUME_MS),
            move || {
                let Some(inner) = weak_inner.upgrade() else {
                    return;
                };

                inner.curious_timeout.borrow_mut().take();
                inner.animator.set_state(MiloState::Idle);
                eprintln!("Milo automatic state: Idle (resume reaction complete)");
            },
        );
        self.inner.curious_timeout.replace(Some(timeout));
    }

    fn cancel_curious_timeout(&self) {
        if let Some(timeout) = self.inner.curious_timeout.borrow_mut().take() {
            timeout.remove();
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

    let timeout_ms = IDLE_TIMEOUT_SECONDS * 1_000;
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
