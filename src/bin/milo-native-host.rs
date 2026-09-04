use milo::browser::{
    browser_socket_path, decode_browser_message, read_native_message, write_local_message,
};
use std::io;
use std::os::unix::net::UnixStream;

fn main() {
    if let Err(error) = run() {
        eprintln!("[milo-native-host] {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let socket_path = browser_socket_path()
        .map_err(|error| format!("could not determine Milo socket path: {error}"))?;
    let mut milo = UnixStream::connect(&socket_path).map_err(|error| {
        format!(
            "could not connect to Milo at {}: {error}",
            socket_path.display()
        )
    })?;
    let stdin = io::stdin();
    let mut input = stdin.lock();

    loop {
        let Some(payload) = read_native_message(&mut input)
            .map_err(|error| format!("invalid native message frame: {error}"))?
        else {
            return Ok(());
        };
        let message = decode_browser_message(&payload)
            .map_err(|error| format!("invalid browser activity message: {error}"))?;
        write_local_message(&mut milo, &message)
            .map_err(|error| format!("could not forward browser activity to Milo: {error}"))?;
    }
}
