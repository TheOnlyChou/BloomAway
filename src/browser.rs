use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;

pub const MAX_NATIVE_MESSAGE_SIZE: usize = 64 * 1024;
pub const MAX_LOCAL_MESSAGE_SIZE: usize = MAX_NATIVE_MESSAGE_SIZE;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BrowserActivity {
    #[serde(rename = "youtube")]
    YouTube,
    #[serde(rename = "youtube_shorts")]
    YouTubeShorts,
    #[serde(rename = "instagram")]
    Instagram,
    #[serde(rename = "instagram_reels")]
    InstagramReels,
    #[serde(rename = "other")]
    Other,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserActivityMessage {
    #[serde(rename = "type")]
    message_type: BrowserMessageType,
    pub activity: BrowserActivity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BrowserCommand {
    #[serde(rename = "close_active_distraction_tab")]
    CloseActiveDistractionTab,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserCommandMessage {
    #[serde(rename = "type")]
    message_type: BrowserCommandMessageType,
    pub command: BrowserCommand,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BrowserCommandResult {
    #[serde(rename = "closed")]
    Closed,
    #[serde(rename = "ignored_not_distracting")]
    IgnoredNotDistracting,
    #[serde(rename = "no_active_tab")]
    NoActiveTab,
    #[serde(rename = "error")]
    Error,
}

impl std::fmt::Display for BrowserCommandResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Closed => "closed",
            Self::IgnoredNotDistracting => "ignored_not_distracting",
            Self::NoActiveTab => "no_active_tab",
            Self::Error => "error",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum FirefoxMessage {
    #[serde(rename = "browser_activity")]
    Activity { activity: BrowserActivity },
    #[serde(rename = "browser_command_result")]
    CommandResult {
        command: BrowserCommand,
        result: BrowserCommandResult,
    },
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum LocalBrowserMessage {
    #[serde(rename = "browser_activity")]
    Activity { activity: BrowserActivity },
    #[serde(rename = "browser_tracking_unavailable")]
    TrackingUnavailable,
    #[serde(rename = "browser_command_result")]
    CommandResult {
        command: BrowserCommand,
        result: BrowserCommandResult,
    },
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
enum BrowserMessageType {
    #[serde(rename = "browser_activity")]
    BrowserActivity,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
enum BrowserCommandMessageType {
    #[serde(rename = "browser_command")]
    BrowserCommand,
}

impl BrowserActivityMessage {
    pub fn new(activity: BrowserActivity) -> Self {
        Self {
            message_type: BrowserMessageType::BrowserActivity,
            activity,
        }
    }
}

impl BrowserCommandMessage {
    pub fn new(command: BrowserCommand) -> Self {
        Self {
            message_type: BrowserCommandMessageType::BrowserCommand,
            command,
        }
    }
}

pub fn browser_socket_path() -> io::Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is missing"))?;

    Ok(PathBuf::from(runtime_dir).join("milo").join("browser.sock"))
}

pub fn decode_browser_message(payload: &[u8]) -> serde_json::Result<BrowserActivityMessage> {
    serde_json::from_slice(payload)
}

pub fn decode_firefox_message(payload: &[u8]) -> serde_json::Result<FirefoxMessage> {
    serde_json::from_slice(payload)
}

pub fn decode_browser_command(payload: &[u8]) -> serde_json::Result<BrowserCommandMessage> {
    serde_json::from_slice(payload)
}

pub fn read_native_message<R: Read>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut length_bytes = [0_u8; 4];
    let first_byte_count = reader.read(&mut length_bytes[..1])?;
    if first_byte_count == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut length_bytes[1..])?;

    let length = u32::from_ne_bytes(length_bytes) as usize;
    if length > MAX_NATIVE_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("native message length {length} exceeds {MAX_NATIVE_MESSAGE_SIZE} bytes"),
        ));
    }

    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

pub fn write_native_message<W: Write>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "native message is too large"))?;
    if payload.len() > MAX_NATIVE_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "native message length {} exceeds {MAX_NATIVE_MESSAGE_SIZE} bytes",
                payload.len()
            ),
        ));
    }

    writer.write_all(&length.to_ne_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

pub fn write_native_json<W: Write, T: Serialize>(writer: &mut W, message: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(message).map_err(io::Error::other)?;
    write_native_message(writer, &payload)
}

pub fn write_local_message<W: Write>(
    writer: &mut W,
    message: &BrowserActivityMessage,
) -> io::Result<()> {
    write_local_json(writer, message)
}

pub fn write_local_tracking_unavailable<W: Write>(writer: &mut W) -> io::Result<()> {
    write_local_json(writer, &LocalBrowserMessage::TrackingUnavailable)
}

pub fn write_local_command<W: Write>(writer: &mut W, command: BrowserCommand) -> io::Result<()> {
    write_local_json(writer, &BrowserCommandMessage::new(command))
}

pub fn write_local_command_result<W: Write>(
    writer: &mut W,
    command: BrowserCommand,
    result: BrowserCommandResult,
) -> io::Result<()> {
    write_local_json(
        writer,
        &LocalBrowserMessage::CommandResult { command, result },
    )
}

fn write_local_json<W: Write, T: Serialize>(writer: &mut W, message: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, message).map_err(io::Error::other)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

pub fn read_local_message<R: BufRead>(reader: &mut R) -> io::Result<Option<LocalBrowserMessage>> {
    read_local_json(reader)
}

pub fn read_local_command<R: BufRead>(reader: &mut R) -> io::Result<Option<BrowserCommandMessage>> {
    read_local_json(reader)
}

fn read_local_json<R: BufRead, T: DeserializeOwned>(reader: &mut R) -> io::Result<Option<T>> {
    let mut payload = Vec::new();
    let byte_count = reader
        .take((MAX_LOCAL_MESSAGE_SIZE + 1) as u64)
        .read_until(b'\n', &mut payload)?;
    if byte_count == 0 {
        return Ok(None);
    }
    if byte_count > MAX_LOCAL_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "local browser message is too large",
        ));
    }
    if payload.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "local browser message ended without a newline",
        ));
    }
    payload.pop();

    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    #[test]
    fn parses_all_browser_activity_strings() {
        let cases = [
            ("youtube", BrowserActivity::YouTube),
            ("youtube_shorts", BrowserActivity::YouTubeShorts),
            ("instagram", BrowserActivity::Instagram),
            ("instagram_reels", BrowserActivity::InstagramReels),
            ("other", BrowserActivity::Other),
        ];

        for (value, expected) in cases {
            let json = format!(r#"{{"type":"browser_activity","activity":"{value}"}}"#);
            assert_eq!(
                decode_browser_message(json.as_bytes()).unwrap().activity,
                expected
            );
        }
    }

    #[test]
    fn rejects_unknown_activity_and_message_types() {
        assert!(
            decode_browser_message(br#"{"type":"browser_activity","activity":"unknown"}"#).is_err()
        );
        assert!(decode_browser_message(br#"{"type":"raw_url","activity":"other"}"#).is_err());
    }

    #[test]
    fn rejects_extra_browser_data() {
        assert!(
            decode_browser_message(
                br#"{"type":"browser_activity","activity":"other","url":"https://example.com"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn serializes_close_active_distraction_tab_command() {
        let message = BrowserCommandMessage::new(BrowserCommand::CloseActiveDistractionTab);

        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            r#"{"type":"browser_command","command":"close_active_distraction_tab"}"#
        );
    }

    #[test]
    fn rejects_invalid_browser_commands() {
        assert!(
            decode_browser_command(
                br#"{"type":"browser_command","command":"close_arbitrary_tab"}"#
            )
            .is_err()
        );
        assert!(
            decode_browser_command(
                br#"{"type":"browser_command","command":"close_active_distraction_tab","url":"https://example.com"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn reads_native_length_framing() {
        let payload = br#"{"type":"browser_activity","activity":"youtube_shorts"}"#;
        let mut framed = Vec::new();
        write_native_message(&mut framed, payload).unwrap();

        assert_eq!(
            read_native_message(&mut Cursor::new(framed)).unwrap(),
            Some(payload.to_vec())
        );
    }

    #[test]
    fn reads_multiple_native_messages_until_eof() {
        let payloads = [
            br#"{"type":"browser_activity","activity":"youtube"}"#.as_slice(),
            br#"{"type":"browser_activity","activity":"youtube_shorts"}"#.as_slice(),
            br#"{"type":"browser_activity","activity":"instagram_reels"}"#.as_slice(),
        ];
        let mut framed = Vec::new();
        for payload in payloads {
            write_native_message(&mut framed, payload).unwrap();
        }

        let mut input = Cursor::new(framed);
        for payload in payloads {
            assert_eq!(
                read_native_message(&mut input).unwrap(),
                Some(payload.to_vec())
            );
        }
        assert_eq!(read_native_message(&mut input).unwrap(), None);
    }

    #[test]
    fn frames_multiple_outgoing_native_commands() {
        let messages = [
            BrowserCommandMessage::new(BrowserCommand::CloseActiveDistractionTab),
            BrowserCommandMessage::new(BrowserCommand::CloseActiveDistractionTab),
        ];
        let mut output = Vec::new();
        for message in &messages {
            write_native_json(&mut output, message).unwrap();
        }

        let mut framed = Cursor::new(output);
        for expected in messages {
            let payload = read_native_message(&mut framed).unwrap().unwrap();
            assert_eq!(decode_browser_command(&payload).unwrap(), expected);
        }
        assert_eq!(read_native_message(&mut framed).unwrap(), None);
    }

    #[test]
    fn native_eof_is_clean() {
        assert_eq!(
            read_native_message(&mut Cursor::new(Vec::new())).unwrap(),
            None
        );
    }

    #[test]
    fn rejects_oversized_native_message_before_reading_payload() {
        let length = (MAX_NATIVE_MESSAGE_SIZE as u32 + 1).to_ne_bytes();
        let error = read_native_message(&mut Cursor::new(length)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn local_protocol_is_newline_delimited_json() {
        let message = BrowserActivityMessage::new(BrowserActivity::InstagramReels);
        let mut encoded = Vec::new();
        write_local_message(&mut encoded, &message).unwrap();
        assert_eq!(encoded.last(), Some(&b'\n'));

        let decoded = read_local_message(&mut BufReader::new(Cursor::new(encoded)))
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded,
            LocalBrowserMessage::Activity {
                activity: BrowserActivity::InstagramReels
            }
        );
    }

    #[test]
    fn local_protocol_carries_tracking_unavailable_explicitly() {
        let mut encoded = Vec::new();
        write_local_tracking_unavailable(&mut encoded).unwrap();

        let decoded = read_local_message(&mut BufReader::new(Cursor::new(encoded)))
            .unwrap()
            .unwrap();
        assert_eq!(decoded, LocalBrowserMessage::TrackingUnavailable);
    }

    #[test]
    fn local_protocol_reads_commands_in_the_reverse_direction() {
        let mut encoded = Vec::new();
        write_local_command(&mut encoded, BrowserCommand::CloseActiveDistractionTab).unwrap();

        let decoded = read_local_command(&mut BufReader::new(Cursor::new(encoded)))
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded,
            BrowserCommandMessage::new(BrowserCommand::CloseActiveDistractionTab)
        );
    }

    #[test]
    fn local_protocol_carries_browser_command_results() {
        let command = BrowserCommand::CloseActiveDistractionTab;
        let result = BrowserCommandResult::IgnoredNotDistracting;
        let mut encoded = Vec::new();
        write_local_command_result(&mut encoded, command, result).unwrap();

        let decoded = read_local_message(&mut BufReader::new(Cursor::new(encoded)))
            .unwrap()
            .unwrap();
        assert_eq!(
            decoded,
            LocalBrowserMessage::CommandResult { command, result }
        );
    }
}
