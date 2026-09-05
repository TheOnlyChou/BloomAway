use milo::browser::{
    BrowserActivityMessage, BrowserCommandMessage, FirefoxMessage, browser_socket_path,
    decode_firefox_message, read_local_command, read_native_message, write_local_command_result,
    write_local_message, write_local_tracking_unavailable, write_native_json,
};
use std::io::{self, BufReader, Read};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

fn main() {
    eprintln!("[milo-native-host] started");
    if let Err(error) = run() {
        eprintln!("[milo-native-host] {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let firefox_writer = start_firefox_writer()?;
    let mut forwarder = MiloForwarder::new(firefox_writer);
    forwarder.connect_if_needed();

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let result = process_firefox_messages(&mut input, |message| match message {
        FirefoxMessage::Activity { activity } => {
            forwarder.send_activity(&BrowserActivityMessage::new(activity));
        }
        FirefoxMessage::CommandResult { command, result } => {
            forwarder.send_command_result(command, result);
        }
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
    F: FnMut(FirefoxMessage),
{
    loop {
        let payload = match read_native_message(input) {
            Ok(Some(payload)) => payload,
            Ok(None) => return Ok(()),
            Err(error) => return Err(format!("invalid native message frame: {error}")),
        };

        match decode_firefox_message(&payload) {
            Ok(message) => forward(message),
            Err(error) => {
                eprintln!("[milo-native-host] invalid Firefox message: {error}");
            }
        }
    }
}

fn start_firefox_writer() -> Result<Sender<BrowserCommandMessage>, String> {
    let (sender, receiver) = mpsc::channel::<BrowserCommandMessage>();
    thread::Builder::new()
        .name("milo-firefox-writer".into())
        .spawn(move || {
            let stdout = io::stdout();
            let mut output = stdout.lock();
            for message in receiver {
                if let Err(error) = write_native_json(&mut output, &message) {
                    eprintln!("[milo-native-host] could not write command to Firefox: {error}");
                    break;
                }
                eprintln!("[milo-native-host] command frame written to Firefox");
            }
        })
        .map_err(|error| format!("could not start Firefox writer: {error}"))?;
    Ok(sender)
}

fn start_milo_command_reader(
    stream: UnixStream,
    firefox_writer: Sender<BrowserCommandMessage>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("milo-command-reader".into())
        .spawn(move || {
            let mut input = BufReader::new(stream);
            loop {
                match read_local_command(&mut input) {
                    Ok(Some(message)) => {
                        eprintln!(
                            "[milo-native-host] local command received: {:?}",
                            message.command
                        );
                        eprintln!("[milo-native-host] forwarding command to Firefox");
                        if firefox_writer.send(message).is_err() {
                            eprintln!("[milo-native-host] Firefox command writer unavailable");
                            let _ = input.get_ref().shutdown(Shutdown::Both);
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        eprintln!("[milo-native-host] invalid local browser command: {error}");
                        let _ = input.get_ref().shutdown(Shutdown::Both);
                        break;
                    }
                }
            }
        })
        .map(|_| ())
}

struct MiloForwarder {
    socket_path: Option<PathBuf>,
    stream: Option<UnixStream>,
    firefox_writer: Sender<BrowserCommandMessage>,
}

impl MiloForwarder {
    fn new(firefox_writer: Sender<BrowserCommandMessage>) -> Self {
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
            firefox_writer,
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
                let command_reader = match stream.try_clone() {
                    Ok(command_reader) => command_reader,
                    Err(error) => {
                        eprintln!("[milo-native-host] could not clone Milo socket: {error}");
                        return false;
                    }
                };
                if let Err(error) =
                    start_milo_command_reader(command_reader, self.firefox_writer.clone())
                {
                    eprintln!("[milo-native-host] could not start Milo command reader: {error}");
                    return false;
                }
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

    fn send_command_result(
        &mut self,
        command: milo::browser::BrowserCommand,
        result: milo::browser::BrowserCommandResult,
    ) {
        self.send_with(|stream| write_local_command_result(stream, command, result));
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
        self.disconnect();

        if self.connect_if_needed() {
            let retry_result = {
                let stream = self.stream.as_mut().expect("reconnected stream is present");
                write(stream)
            };
            if let Err(error) = retry_result {
                eprintln!("[milo-native-host] could not forward to Milo after reconnect: {error}");
                self.disconnect();
            }
        }
    }

    fn disconnect(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milo::browser::{
        BrowserActivity, BrowserCommand, BrowserCommandResult, write_native_message,
    };
    use std::io::Cursor;

    #[test]
    fn processes_sequential_activity_and_result_messages_before_eof() {
        let inputs = [
            br#"{"type":"browser_activity","activity":"youtube"}"#.as_slice(),
            br#"{"type":"browser_command_result","command":"close_active_distraction_tab","result":"closed"}"#.as_slice(),
            br#"{"type":"browser_activity","activity":"instagram_reels"}"#.as_slice(),
        ];
        let mut framed = Vec::new();
        for input in inputs {
            write_native_message(&mut framed, input).unwrap();
        }

        let mut messages = Vec::new();
        process_firefox_messages(&mut Cursor::new(framed), |message| {
            messages.push(message);
        })
        .unwrap();

        assert_eq!(
            messages,
            [
                FirefoxMessage::Activity {
                    activity: BrowserActivity::YouTube,
                },
                FirefoxMessage::CommandResult {
                    command: BrowserCommand::CloseActiveDistractionTab,
                    result: BrowserCommandResult::Closed,
                },
                FirefoxMessage::Activity {
                    activity: BrowserActivity::InstagramReels,
                },
            ]
        );
    }
}
