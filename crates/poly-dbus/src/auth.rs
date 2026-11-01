//! The authentication exchange before messages flow (the D-Bus
//! Specification, "Authentication Protocol"): the client sends one NUL
//! byte, then `\r\n`-ended lines; the server answers. Poly's bus offers
//! `EXTERNAL` (the client's user ID in hexadecimal ASCII digits, checked
//! against the socket's peer credentials when the caller knows them) and
//! `ANONYMOUS`, answers `NEGOTIATE_UNIX_FD` with `ERROR` (no descriptors
//! on this bus), and after `BEGIN` the rest of the stream is messages.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The server side of one connection's exchange.
#[derive(Debug)]
pub struct ServerAuth {
    /// The bus's GUID, 32 hexadecimal digits.
    guid: String,
    /// The peer's user ID from the socket, if known.
    peer_uid: Option<u32>,
    state: State,
    line: Vec<u8>,
    out: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for the NUL byte.
    Nul,
    /// Waiting for AUTH.
    Auth,
    /// AUTH EXTERNAL without an initial response: waiting for DATA.
    Data,
    /// OK sent: waiting for BEGIN (or NEGOTIATE_UNIX_FD).
    Ok,
    Begun,
    Failed,
}

/// What a chunk of bytes did to the exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// More lines are needed.
    Pending,
    /// `BEGIN` came: these bytes after it are the first messages.
    Begun(Vec<u8>),
    /// The client broke the protocol: disconnect.
    Failed,
}

/// The longest line accepted.
const MAX_LINE: usize = 16 * 1024;

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// `bytes` as lowercase hexadecimal.
pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl ServerAuth {
    pub fn new(guid: &str, peer_uid: Option<u32>) -> ServerAuth {
        ServerAuth { guid: guid.into(), peer_uid, state: State::Nul, line: Vec::new(), out: Vec::new() }
    }

    /// The lines to send.
    pub fn take_output(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Begun
    }

    /// Queue a line for the client.
    fn reply(&mut self, line: &str) {
        self.out.extend_from_slice(line.as_bytes());
        self.out.extend_from_slice(b"\r\n");
    }

    fn rejected(&mut self) {
        self.reply("REJECTED EXTERNAL ANONYMOUS");
        self.state = State::Auth;
    }

    /// Whether `response` (the hex-encoded EXTERNAL initial response)
    /// names a user the socket allows.
    fn external_ok(&self, response: &str) -> bool {
        if response.is_empty() {
            // No identity given: the socket's credentials decide.
            return true;
        }
        let Some(text) = hex_decode(response).and_then(|b| String::from_utf8(b).ok()) else { return false };
        let Ok(uid) = text.parse::<u32>() else { return false };
        self.peer_uid.is_none_or(|p| p == uid)
    }

    fn ok(&mut self) {
        let line = format!("OK {}", self.guid);
        self.reply(&line);
        self.state = State::Ok;
    }

    fn command(&mut self, line: &str) {
        let mut words = line.split(' ');
        let cmd = words.next().unwrap_or("");
        let args: Vec<&str> = words.collect();
        match (self.state, cmd) {
            (State::Auth, "AUTH") => match args.first().copied() {
                Some("EXTERNAL") => match args.get(1) {
                    Some(r) if self.external_ok(r) => self.ok(),
                    Some(_) => self.rejected(),
                    None => {
                        self.reply("DATA");
                        self.state = State::Data;
                    }
                },
                Some("ANONYMOUS") => self.ok(),
                _ => self.rejected(),
            },
            (State::Data, "DATA") => {
                if self.external_ok(args.first().copied().unwrap_or("")) {
                    self.ok();
                } else {
                    self.rejected();
                }
            }
            (State::Data | State::Ok, "CANCEL" | "ERROR") => self.rejected(),
            (State::Ok, "NEGOTIATE_UNIX_FD") => self.reply("ERROR \"file descriptors are not passed on this bus\""),
            (State::Ok, "BEGIN") => self.state = State::Begun,
            (State::Auth, "BEGIN") => self.state = State::Failed,
            (State::Auth, "CANCEL" | "ERROR") => self.rejected(),
            _ => self.reply("ERROR \"unknown command\""),
        }
    }

    /// Bytes from the client.
    pub fn receive(&mut self, bytes: &[u8]) -> Progress {
        let mut i = 0;
        while i < bytes.len() {
            match self.state {
                State::Failed => return Progress::Failed,
                State::Begun => return Progress::Begun(bytes[i..].to_vec()),
                State::Nul => {
                    if bytes[i] != 0 {
                        self.state = State::Failed;
                        return Progress::Failed;
                    }
                    self.state = State::Auth;
                    i += 1;
                }
                _ => {
                    self.line.push(bytes[i]);
                    i += 1;
                    if self.line.ends_with(b"\r\n") {
                        let line = core::mem::take(&mut self.line);
                        let text = String::from_utf8_lossy(&line[..line.len() - 2]).into_owned();
                        self.command(&text);
                    } else if self.line.len() > MAX_LINE {
                        self.state = State::Failed;
                    }
                }
            }
        }
        match self.state {
            State::Failed => Progress::Failed,
            State::Begun => Progress::Begun(Vec::new()),
            _ => Progress::Pending,
        }
    }
}

/// What a client sends to authenticate as user `uid` with EXTERNAL, up to
/// and including BEGIN (to be sent after the server's `OK`): the NUL and
/// the AUTH line, then the BEGIN line.
pub fn client_lines(uid: u32) -> (Vec<u8>, Vec<u8>) {
    let mut auth = Vec::from([0u8]);
    auth.extend_from_slice(format!("AUTH EXTERNAL {}\r\n", hex_encode(format!("{uid}").as_bytes())).as_bytes());
    (auth, b"BEGIN\r\n".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn external_with_the_peers_uid() {
        let mut a = ServerAuth::new(GUID, Some(1000));
        let (auth, begin) = client_lines(1000);
        assert_eq!(auth, b"\0AUTH EXTERNAL 31303030\r\n");
        assert_eq!(a.receive(&auth), Progress::Pending);
        assert_eq!(a.take_output(), format!("OK {GUID}\r\n").as_bytes());
        assert_eq!(a.receive(b"NEGOTIATE_UNIX_FD\r\n"), Progress::Pending);
        assert!(a.take_output().starts_with(b"ERROR"));
        let mut rest = begin.clone();
        rest.extend_from_slice(b"l\x01");
        assert_eq!(a.receive(&rest), Progress::Begun(b"l\x01".to_vec()));
        assert!(a.is_done());
    }

    #[test]
    fn rejections_and_data() {
        // Another user than the socket's: rejected, may try again.
        let mut a = ServerAuth::new(GUID, Some(0));
        a.receive(b"\0AUTH EXTERNAL 31303030\r\n");
        assert_eq!(a.take_output(), b"REJECTED EXTERNAL ANONYMOUS\r\n");
        // Plain AUTH lists the mechanisms.
        a.receive(b"AUTH\r\n");
        assert_eq!(a.take_output(), b"REJECTED EXTERNAL ANONYMOUS\r\n");
        // EXTERNAL without a response, then DATA.
        a.receive(b"AUTH EXTERNAL\r\n");
        assert_eq!(a.take_output(), b"DATA\r\n");
        a.receive(b"DATA 30\r\n");
        assert!(a.take_output().starts_with(b"OK "));
        // Lines may come in pieces.
        let mut b = ServerAuth::new(GUID, None);
        for chunk in [&b"\0AU"[..], b"TH ANONYM", b"OUS\r", b"\nBEG", b"IN\r\n"] {
            b.receive(chunk);
        }
        assert!(b.is_done());
        // No NUL first, or BEGIN before AUTH: disconnect.
        assert_eq!(ServerAuth::new(GUID, None).receive(b"AUTH\r\n"), Progress::Failed);
        assert_eq!(ServerAuth::new(GUID, None).receive(b"\0BEGIN\r\n"), Progress::Failed);
    }
}
