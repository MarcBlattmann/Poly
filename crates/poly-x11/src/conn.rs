//! A client connection without I/O.
//!
//! [`Connection::new`] queues the setup request; the caller sends what
//! [`Connection::take_output`] returns and feeds what it reads to
//! [`Connection::feed`], then takes [`Incoming`] items until there are
//! none. Requests are numbered from 1 in the order they are queued
//! ([`Connection::send`] returns the number); the server echoes the low 16
//! bits in every reply, error and event, which [`Connection::poll`] widens
//! back to the full number by the last request sent (the protocol has
//! the server handle requests in order, so a reply is never for a request
//! more than 65535 behind the last one).
//!
//! Resource IDs come from the setup's base and mask ("Resource IDs").

use crate::event::{Event, Message, XError, next_message};
use crate::request::Request;
use crate::setup::{self, Setup};
use alloc::string::String;
use alloc::vec::Vec;

/// What came from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// The connection is up.
    Setup(Setup),
    /// The server refused the connection.
    Refused(String),
    /// A reply to request number `sequence`, all of its bytes.
    Reply { sequence: u64, bytes: Vec<u8> },
    /// An error for request number `sequence`.
    Error { sequence: u64, error: XError },
    /// An event (`send_event`: a client made it with `SendEvent`).
    Event { event: Event, send_event: bool },
}

/// A broken stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Broken(pub &'static str);

#[derive(Debug, PartialEq, Eq)]
enum State {
    Setup,
    Ready,
    Closed,
}

/// The connection.
#[derive(Debug)]
pub struct Connection {
    state: State,
    out: Vec<u8>,
    input: Vec<u8>,
    /// The number of the last request queued.
    sequence: u64,
    id_base: u32,
    id_mask: u32,
    /// The next resource ID's index (shifted into the mask's bits).
    next_id: u32,
}

impl Default for Connection {
    fn default() -> Self {
        Connection::new()
    }
}

impl Connection {
    /// A connection with the setup request queued.
    pub fn new() -> Connection {
        let mut out = Vec::new();
        setup::request(&mut out);
        Connection { state: State::Setup, out, input: Vec::new(), sequence: 0, id_base: 0, id_mask: 0, next_id: 1 }
    }

    /// Whether the setup succeeded.
    pub fn is_ready(&self) -> bool {
        self.state == State::Ready
    }

    /// The number of the last request queued.
    pub fn last_sequence(&self) -> u64 {
        self.sequence
    }

    /// Queue a request; returns its number.
    pub fn send(&mut self, request: &Request<'_>) -> u64 {
        request.encode(&mut self.out);
        self.sequence += 1;
        self.sequence
    }

    /// The bytes to send (taken).
    pub fn take_output(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }

    /// Whether there is something to send.
    pub fn has_output(&self) -> bool {
        !self.out.is_empty()
    }

    /// Bytes read from the server.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.input.extend_from_slice(bytes);
    }

    /// A new resource ID, or `None` when the range is used up.
    pub fn generate_id(&mut self) -> Option<u32> {
        if self.id_mask == 0 {
            return None;
        }
        let shift = self.id_mask.trailing_zeros();
        let index = self.next_id;
        let id = index.checked_shl(shift).filter(|v| v & !self.id_mask == 0 && v >> shift == index)?;
        self.next_id += 1;
        Some(self.id_base | id)
    }

    /// The full number of the request whose low 16 bits are `low`: the
    /// latest one queued with those bits.
    fn widen(&self, low: u16) -> u64 {
        let behind = (self.sequence as u16).wrapping_sub(low);
        self.sequence.saturating_sub(u64::from(behind))
    }

    /// The next thing the server sent, if all of it is there.
    pub fn poll(&mut self) -> Result<Option<Incoming>, Broken> {
        match self.state {
            State::Closed => Err(Broken("the connection was refused")),
            State::Setup => {
                let Some((reply, used)) = setup::parse(&self.input).map_err(|e| Broken(e.0))? else {
                    return Ok(None);
                };
                self.input.drain(..used);
                Ok(Some(match reply {
                    setup::Reply::Success(s) => {
                        self.state = State::Ready;
                        (self.id_base, self.id_mask) = (s.resource_id_base, s.resource_id_mask);
                        Incoming::Setup(s)
                    }
                    setup::Reply::Failed(why) | setup::Reply::Authenticate(why) => {
                        self.state = State::Closed;
                        Incoming::Refused(why)
                    }
                }))
            }
            State::Ready => {
                let Some((msg, used)) = next_message(&self.input) else { return Ok(None) };
                self.input.drain(..used);
                Ok(Some(match msg {
                    Message::Reply { sequence, bytes } => Incoming::Reply { sequence: self.widen(sequence), bytes },
                    Message::Error(error) => Incoming::Error { sequence: self.widen(error.sequence), error },
                    Message::Event { event, send_event, .. } => Incoming::Event { event, send_event },
                    Message::Generic { .. } => return self.poll(),
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::encode_reply;
    use crate::setup::{Screen, encode_success};
    use alloc::vec;

    fn ready() -> Connection {
        let mut c = Connection::new();
        assert_eq!(c.take_output().len(), 12, "the setup request");
        let setup = Setup {
            resource_id_base: 0x0060_0000,
            resource_id_mask: 0x001f_ffff,
            screens: vec![Screen { root: 0x3b6, width: 1280, height: 800, ..Screen::default() }],
            ..Setup::default()
        };
        c.feed(&encode_success(&setup));
        assert!(matches!(c.poll(), Ok(Some(Incoming::Setup(s))) if s.screens[0].root == 0x3b6));
        assert!(c.is_ready());
        c
    }

    #[test]
    fn numbers_requests_and_matches_replies() {
        let mut c = ready();
        assert_eq!(c.send(&Request::InternAtom { only_if_exists: false, name: b"A" }), 1);
        assert_eq!(c.send(&Request::MapWindow { window: 5 }), 2);
        assert_eq!(c.take_output().len(), 12 + 8);
        let mut body = vec![0u8; 24];
        body[0] = 0x55;
        let reply = encode_reply(0, 1, &body);
        // In two pieces.
        c.feed(&reply[..10]);
        assert_eq!(c.poll(), Ok(None));
        c.feed(&reply[10..]);
        match c.poll() {
            Ok(Some(Incoming::Reply { sequence: 1, bytes })) => assert_eq!(bytes[8], 0x55),
            other => panic!("{other:?}"),
        }
        assert_eq!(c.poll(), Ok(None));
    }

    #[test]
    fn widens_sequence_numbers_past_sixteen_bits() {
        let mut c = ready();
        for _ in 0..70_000 {
            c.send(&Request::GetInputFocus);
        }
        c.take_output();
        let e = XError { code: 3, sequence: (69_999u64 & 0xffff) as u16, value: 0, minor_opcode: 0, major_opcode: 43 };
        c.feed(&crate::event::encode_error(&e));
        assert!(matches!(c.poll(), Ok(Some(Incoming::Error { sequence: 69_999, .. }))));
        let ev = Event::MapRequest { parent: 1, window: 2 }.encode(4);
        c.feed(&ev);
        assert_eq!(
            c.poll(),
            Ok(Some(Incoming::Event { event: Event::MapRequest { parent: 1, window: 2 }, send_event: false }))
        );
    }

    #[test]
    fn generates_ids_inside_the_mask() {
        let mut c = ready();
        assert_eq!(c.generate_id(), Some(0x0060_0001));
        assert_eq!(c.generate_id(), Some(0x0060_0002));
        let mut c = Connection::new();
        assert_eq!(c.generate_id(), None, "not before the setup");
        c.id_base = 0x100;
        c.id_mask = 0xf0;
        c.next_id = 15;
        assert_eq!(c.generate_id(), Some(0x1f0));
        assert_eq!(c.generate_id(), None, "the range is used up");
    }

    #[test]
    fn a_refused_connection_stays_closed() {
        let mut c = Connection::new();
        c.feed(&[0, 3, 11, 0, 0, 0, 1, 0, b'b', b'a', b'd', 0]);
        assert_eq!(c.poll(), Ok(Some(Incoming::Refused("bad".into()))));
        assert!(c.poll().is_err());
    }
}
