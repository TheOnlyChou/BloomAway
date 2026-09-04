use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use gtk4::glib;
use milo::browser::{BrowserActivity, browser_socket_path, read_local_message};
use std::fs;
use std::io::{self, BufReader};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;

pub fn start_browser_activity_monitor<F>(mut on_browser_activity: F)
where
    F: FnMut(BrowserActivity) + 'static,
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
            if let Err(error) = run_browser_listener(sender) {
                eprintln!("[milo] browser activity unavailable: {error}");
            }
        })
    {
        eprintln!("[milo] browser activity unavailable: failed to start listener: {error}");
    }
}

fn run_browser_listener(sender: UnboundedSender<BrowserActivity>) -> io::Result<()> {
    let socket_path = browser_socket_path()?;
    let listener = bind_local_socket(&socket_path)?;

    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(error) = forward_browser_messages(stream, &sender) {
                    eprintln!("[milo] browser connection closed: {error}");
                }
                if sender.is_closed() {
                    return Ok(());
                }
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

fn forward_browser_messages(
    stream: UnixStream,
    sender: &UnboundedSender<BrowserActivity>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    while let Some(message) = read_local_message(&mut reader)? {
        if sender.unbounded_send(message.activity).is_err() {
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
