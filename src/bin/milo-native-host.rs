use milo::browser::{
    BrowserActivityMessage, browser_socket_path, decode_browser_message, read_native_message,
    write_local_message, write_local_tracking_unavailable,
};
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn main() {
    eprintln!("[milo-native-host] started");
    if let Err(error) = run() {
        eprintln!("[milo-native-host] {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut forwarder = MiloForwarder::new();
    forwarder.connect_if_needed();

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let result = process_firefox_messages(&mut input, |message| {
        forwarder.send_activity(message);
    });

    if result.is_ok() {
        eprintln!("[milo-native-host] Firefox connection closed");
    }
    forwarder.send_tracking_unavailable();
    result
}

fn process_firefox_messages<R, F>(input: &mut R, mut forward: F) -> Result<(), String>
where
    R: Read,
    F: FnMut(&BrowserActivityMessage),
{
    loop {
        let payload = match read_native_message(input) {
            Ok(Some(payload)) => payload,
            Ok(None) => return Ok(()),
            Err(error) => return Err(format!("invalid native message frame: {error}")),
        };

        match decode_browser_message(&payload) {
            Ok(message) => forward(&message),
            Err(error) => {
                eprintln!("[milo-native-host] invalid browser activity message: {error}");
            }
        }
    }
}

struct MiloForwarder {
    socket_path: Option<PathBuf>,
    stream: Option<UnixStream>,
}

impl MiloForwarder {
    fn new() -> Self {
        let socket_path = match browser_socket_path() {
            Ok(path) => Some(path),
            Err(error) => {
                eprintln!("[milo-native-host] could not determine Milo socket path: {error}");
                None
            }
        };
        Self {
            socket_path,
            stream: None,
        }
    }

    fn connect_if_needed(&mut self) -> bool {
        if self.stream.is_some() {
            return true;
        }

        let Some(socket_path) = self.socket_path.as_ref() else {
            return false;
        };
        match UnixStream::connect(socket_path) {
            Ok(stream) => {
                self.stream = Some(stream);
                eprintln!("[milo-native-host] connected to Milo");
                true
            }
            Err(error) => {
                eprintln!(
                    "[milo-native-host] could not connect to Milo at {}: {error}",
                    socket_path.display()
                );
                false
            }
        }
    }

    fn send_activity(&mut self, message: &BrowserActivityMessage) {
        self.send_with(|stream| write_local_message(stream, message));
    }

    fn send_tracking_unavailable(&mut self) {
        self.send_with(write_local_tracking_unavailable);
    }

    fn send_with<F>(&mut self, mut write: F)
    where
        F: FnMut(&mut UnixStream) -> io::Result<()>,
    {
        if !self.connect_if_needed() {
            return;
        }

        let first_error = {
            let stream = self.stream.as_mut().expect("connected stream is present");
            write(stream).err()
        };
        let Some(first_error) = first_error else {
            return;
        };

        eprintln!("[milo-native-host] could not forward to Milo: {first_error}");
        self.stream = None;

        if self.connect_if_needed() {
            let retry_result = {
                let stream = self.stream.as_mut().expect("reconnected stream is present");
                write(stream)
            };
            if let Err(error) = retry_result {
                eprintln!("[milo-native-host] could not forward to Milo after reconnect: {error}");
                self.stream = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milo::browser::{BrowserActivity, write_native_message};
    use std::io::Cursor;

    #[test]
    fn processes_every_native_message_before_eof() {
        let inputs = [
            br#"{"type":"browser_activity","activity":"youtube"}"#.as_slice(),
            br#"{"type":"browser_activity","activity":"youtube_shorts"}"#.as_slice(),
            br#"{"type":"browser_activity","activity":"instagram_reels"}"#.as_slice(),
        ];
        let mut framed = Vec::new();
        for input in inputs {
            write_native_message(&mut framed, input).unwrap();
        }

        let mut activities = Vec::new();
        process_firefox_messages(&mut Cursor::new(framed), |message| {
            activities.push(message.activity);
        })
        .unwrap();

        assert_eq!(
            activities,
            [
                BrowserActivity::YouTube,
                BrowserActivity::YouTubeShorts,
                BrowserActivity::InstagramReels,
            ]
        );
    }
}
