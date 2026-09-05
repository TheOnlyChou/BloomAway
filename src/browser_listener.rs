use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use gtk4::glib;
use milo::browser::{
    BrowserActivity, BrowserCommand, BrowserCommandResult, LocalBrowserMessage,
    browser_socket_path, read_local_message, write_local_command,
};
use std::fs;
use std::io::{self, BufReader, Read};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserActivityEvent {
    Activity(BrowserActivity),
    CommandResult {
        command: BrowserCommand,
        result: BrowserCommandResult,
    },
    Unavailable,
}

#[derive(Debug)]
pub enum BrowserCommandSendError {
    Unavailable,
    Transport(io::Error),
}

impl std::fmt::Display for BrowserCommandSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("native host is unavailable"),
            Self::Transport(error) => write!(formatter, "local transport failed: {error}"),
        }
    }
}

#[derive(Clone, Default)]
pub struct BrowserBridge {
    writer: Arc<Mutex<Option<UnixStream>>>,
}

impl BrowserBridge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn send(&self, command: BrowserCommand) -> Result<(), BrowserCommandSendError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| BrowserCommandSendError::Unavailable)?;
        let result = match writer.as_mut() {
            Some(stream) => write_local_command(stream, command),
            None => return Err(BrowserCommandSendError::Unavailable),
        };
        if let Err(error) = result {
            *writer = None;
            return Err(BrowserCommandSendError::Transport(error));
        }
        Ok(())
    }

    fn attach(&self, stream: &UnixStream) -> io::Result<()> {
        let command_writer = stream.try_clone()?;
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("browser bridge lock is poisoned"))?;
        *writer = Some(command_writer);
        Ok(())
    }

    fn detach(&self) {
        match self.writer.lock() {
            Ok(mut writer) => *writer = None,
            Err(_) => eprintln!("[milo] browser bridge lock is poisoned"),
        }
    }
}

pub fn start_browser_activity_monitor<F>(bridge: BrowserBridge, mut on_browser_activity: F)
where
    F: FnMut(BrowserActivityEvent) + 'static,
{
    let (sender, mut receiver) = unbounded();

    glib::MainContext::default().spawn_local(async move {
        while let Some(activity) = receiver.next().await {
            on_browser_activity(activity);
        }
    });

    if let Err(error) = thread::Builder::new()
        .name("milo-browser-activity".into())
        .spawn(move || {
            if let Err(error) = run_browser_listener(sender, bridge) {
                eprintln!("[milo] browser activity unavailable: {error}");
            }
        })
    {
        eprintln!("[milo] browser activity unavailable: failed to start listener: {error}");
    }
}

fn run_browser_listener(
    sender: UnboundedSender<BrowserActivityEvent>,
    bridge: BrowserBridge,
) -> io::Result<()> {
    let socket_path = browser_socket_path()?;
    let listener = bind_local_socket(&socket_path)?;

    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(error) = bridge.attach(&stream) {
                    eprintln!("[milo] browser command unavailable: {error}");
                }
                if let Err(error) = forward_browser_messages(stream, &sender) {
                    eprintln!("[milo] browser connection closed: {error}");
                }
                bridge.detach();
            }
            Err(error) => eprintln!("[milo] could not accept browser connection: {error}"),
        }
    }

    Ok(())
}

fn bind_local_socket(socket_path: &Path) -> io::Result<UnixListener> {
    let parent = socket_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent"))?;
    create_private_runtime_directory(parent)?;
    match fs::symlink_metadata(socket_path) {
        Ok(_) => remove_stale_socket(socket_path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    match UnixListener::bind(socket_path) {
        Ok(listener) => secure_listener(listener, socket_path),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            remove_stale_socket(socket_path)?;
            secure_listener(UnixListener::bind(socket_path)?, socket_path)
        }
        Err(error) => Err(error),
    }
}

fn create_private_runtime_directory(path: &Path) -> io::Result<()> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a real directory", path.display()),
            ));
        }
    } else {
        fs::create_dir_all(path)?;
    }

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

fn remove_stale_socket(socket_path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(socket_path)?;
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "refusing to remove non-socket path {}",
                socket_path.display()
            ),
        ));
    }

    match UnixStream::connect(socket_path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("{} already has a live listener", socket_path.display()),
            ));
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) => {}
        Err(error) => return Err(error),
    }

    fs::remove_file(socket_path)
}

fn secure_listener(listener: UnixListener, socket_path: &Path) -> io::Result<UnixListener> {
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn forward_browser_messages<R: Read>(
    stream: R,
    sender: &UnboundedSender<BrowserActivityEvent>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    while let Some(message) = read_local_message(&mut reader)? {
        let event = match message {
            LocalBrowserMessage::Activity { activity } => BrowserActivityEvent::Activity(activity),
            LocalBrowserMessage::TrackingUnavailable => BrowserActivityEvent::Unavailable,
            LocalBrowserMessage::CommandResult { command, result } => {
                BrowserActivityEvent::CommandResult { command, result }
            }
        };
        if sender.unbounded_send(event).is_err() {
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use milo::browser::{
        BrowserActivityMessage, write_local_message, write_local_tracking_unavailable,
    };
    use std::io::Cursor;

    #[test]
    fn local_stream_eof_does_not_mean_tracking_unavailable() {
        let mut encoded = Vec::new();
        write_local_message(
            &mut encoded,
            &BrowserActivityMessage::new(BrowserActivity::YouTubeShorts),
        )
        .unwrap();

        let (sender, mut receiver) = unbounded();
        forward_browser_messages(Cursor::new(encoded), &sender).unwrap();
        drop(sender);

        assert_eq!(
            receiver.try_recv().unwrap(),
            BrowserActivityEvent::Activity(BrowserActivity::YouTubeShorts)
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn explicit_tracking_unavailable_message_is_forwarded() {
        let mut encoded = Vec::new();
        write_local_tracking_unavailable(&mut encoded).unwrap();

        let (sender, mut receiver) = unbounded();
        forward_browser_messages(Cursor::new(encoded), &sender).unwrap();
        drop(sender);

        assert_eq!(
            receiver.try_recv().unwrap(),
            BrowserActivityEvent::Unavailable
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn sending_without_a_native_host_is_unavailable_and_not_queued() {
        let bridge = BrowserBridge::new();

        assert!(matches!(
            bridge.send(BrowserCommand::CloseActiveDistractionTab),
            Err(BrowserCommandSendError::Unavailable)
        ));
        assert!(bridge.writer.lock().unwrap().is_none());
    }

    #[test]
    fn refuses_to_replace_a_non_socket_path() {
        let unique = format!(
            "milo-browser-listener-test-{}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed"),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        fs::create_dir(&directory).unwrap();
        let socket_path = directory.join("browser.sock");
        fs::write(&socket_path, b"not a socket").unwrap();

        let error = bind_local_socket(&socket_path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        fs::remove_file(socket_path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
