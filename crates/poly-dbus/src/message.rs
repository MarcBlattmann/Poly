//! D-Bus messages (the D-Bus Specification, "Message Format"): a header —
//! byte order, type, flags, protocol version 1, body length, serial, and
//! an array of `(code, variant)` header fields — padded to 8, then the
//! body, whose signature is the `SIGNATURE` field.

use crate::wire::{self, Error, Reader, Result, Value, Writer};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// The longest message the specification allows.
pub const MAX_MESSAGE: usize = 128 << 20;

/// `NO_REPLY_EXPECTED`: the caller wants no method return or error.
pub const NO_REPLY_EXPECTED: u8 = 0x1;
/// `NO_AUTO_START`: do not start a service to receive this.
pub const NO_AUTO_START: u8 = 0x2;

/// The header field codes.
mod field {
    pub const PATH: u8 = 1;
    pub const INTERFACE: u8 = 2;
    pub const MEMBER: u8 = 3;
    pub const ERROR_NAME: u8 = 4;
    pub const REPLY_SERIAL: u8 = 5;
    pub const DESTINATION: u8 = 6;
    pub const SENDER: u8 = 7;
    pub const SIGNATURE: u8 = 8;
    pub const UNIX_FDS: u8 = 9;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    MethodCall = 1,
    MethodReturn = 2,
    Error = 3,
    Signal = 4,
}

/// A message, decoded.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub kind: MessageType,
    pub flags: u8,
    pub serial: u32,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub error_name: Option<String>,
    pub reply_serial: Option<u32>,
    pub destination: Option<String>,
    pub sender: Option<String>,
    pub body: Vec<Value>,
    pub unix_fds: u32,
}

impl Message {
    fn new(kind: MessageType) -> Message {
        Message {
            kind,
            flags: 0,
            serial: 0,
            path: None,
            interface: None,
            member: None,
            error_name: None,
            reply_serial: None,
            destination: None,
            sender: None,
            body: Vec::new(),
            unix_fds: 0,
        }
    }

    /// A method call of `member` on `path` (of `interface`, if given), to
    /// `destination` (a bus name).
    pub fn method_call(
        destination: Option<&str>,
        path: &str,
        interface: Option<&str>,
        member: &str,
        body: Vec<Value>,
    ) -> Message {
        Message {
            destination: destination.map(String::from),
            path: Some(path.into()),
            interface: interface.map(String::from),
            member: Some(member.into()),
            body,
            ..Message::new(MessageType::MethodCall)
        }
    }

    /// A signal `interface.member` from `path`.
    pub fn signal(path: &str, interface: &str, member: &str, body: Vec<Value>) -> Message {
        Message {
            path: Some(path.into()),
            interface: Some(interface.into()),
            member: Some(member.into()),
            body,
            ..Message::new(MessageType::Signal)
        }
    }

    /// The return of method call `call`, to its sender.
    pub fn method_return(call: &Message, body: Vec<Value>) -> Message {
        Message {
            reply_serial: Some(call.serial),
            destination: call.sender.clone(),
            body,
            ..Message::new(MessageType::MethodReturn)
        }
    }

    /// Error `name` (with a message for people) for method call `call`.
    pub fn error(call: &Message, name: &str, text: &str) -> Message {
        Message {
            error_name: Some(name.into()),
            reply_serial: Some(call.serial),
            destination: call.sender.clone(),
            body: vec![Value::str(text)],
            ..Message::new(MessageType::Error)
        }
    }

    /// Whether the sender of this method call wants a reply.
    pub fn wants_reply(&self) -> bool {
        self.kind == MessageType::MethodCall && self.flags & NO_REPLY_EXPECTED == 0
    }

    /// The body's signature.
    pub fn signature(&self) -> String {
        self.body.iter().map(Value::signature).collect()
    }

    /// `interface.member`, for logs.
    pub fn name(&self) -> String {
        let i = self.interface.as_deref().unwrap_or("");
        let m = self.member.as_deref().or(self.error_name.as_deref()).unwrap_or("");
        alloc::format!("{i}.{m}")
    }

    /// The message's bytes (little-endian), with serial `serial`.
    pub fn encode(&self, serial: u32) -> Vec<u8> {
        let body = wire::marshal(&self.body);
        let mut fields = Vec::new();
        let mut add = |code: u8, v: Value| {
            fields.push(Value::Struct(vec![Value::Byte(code), Value::Variant(Box::new(v))]));
        };
        let opt = |s: &Option<String>| s.clone();
        if let Some(p) = opt(&self.path) {
            add(field::PATH, Value::Path(p));
        }
        if let Some(s) = opt(&self.interface) {
            add(field::INTERFACE, Value::Str(s));
        }
        if let Some(s) = opt(&self.member) {
            add(field::MEMBER, Value::Str(s));
        }
        if let Some(s) = opt(&self.error_name) {
            add(field::ERROR_NAME, Value::Str(s));
        }
        if let Some(r) = self.reply_serial {
            add(field::REPLY_SERIAL, Value::Uint32(r));
        }
        if let Some(s) = opt(&self.destination) {
            add(field::DESTINATION, Value::Str(s));
        }
        if let Some(s) = opt(&self.sender) {
            add(field::SENDER, Value::Str(s));
        }
        let sig = self.signature();
        if !sig.is_empty() {
            add(field::SIGNATURE, Value::Signature(sig));
        }
        if self.unix_fds > 0 {
            add(field::UNIX_FDS, Value::Uint32(self.unix_fds));
        }
        let mut w = Writer::new();
        w.buf.extend_from_slice(&[b'l', self.kind as u8, self.flags, 1]);
        w.u32(body.len() as u32);
        w.u32(serial);
        w.value(&Value::Array("(yv)".into(), fields));
        w.pad(8);
        w.buf.extend_from_slice(&body);
        w.buf
    }

    /// The whole length of the message whose bytes start `prefix`, once its
    /// first 16 bytes are there (`None` before).
    pub fn length(prefix: &[u8]) -> Option<Result<usize>> {
        if prefix.len() < 16 {
            return None;
        }
        let big = match prefix[0] {
            b'l' => false,
            b'B' => true,
            _ => return Some(Err(Error("unknown byte order"))),
        };
        let word = |at: usize| {
            let b: [u8; 4] = prefix[at..at + 4].try_into().unwrap_or([0; 4]);
            if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
        };
        let (body, fields) = (word(4) as usize, word(12) as usize);
        let total = (16 + fields).next_multiple_of(8) + body;
        Some(if total > MAX_MESSAGE { Err(Error("message too long")) } else { Ok(total) })
    }

    /// Decode one whole message.
    pub fn decode(bytes: &[u8]) -> Result<Message> {
        let len = Message::length(bytes).ok_or(Error("message too short"))??;
        if bytes.len() != len {
            return Err(Error("not one whole message"));
        }
        let big = bytes[0] == b'B';
        let kind = match bytes[1] {
            1 => MessageType::MethodCall,
            2 => MessageType::MethodReturn,
            3 => MessageType::Error,
            4 => MessageType::Signal,
            _ => return Err(Error("unknown message type")),
        };
        if bytes[3] != 1 {
            return Err(Error("unknown protocol version"));
        }
        let mut r = Reader::at(bytes, 4, big);
        let body_len = r.u32()? as usize;
        let serial = r.u32()?;
        if serial == 0 {
            return Err(Error("serial 0"));
        }
        let fields = r.value("a(yv)")?;
        let mut m = Message { flags: bytes[2], serial, ..Message::new(kind) };
        let mut signature = String::new();
        for f in fields.as_array().unwrap_or(&[]) {
            let Value::Struct(parts) = f else { continue };
            let (Some(Value::Byte(code)), Some(v)) = (parts.first(), parts.get(1)) else { continue };
            let v = v.unvariant();
            let text = || v.as_str().map(String::from).ok_or(Error("a header field of the wrong type"));
            match *code {
                field::PATH if matches!(v, Value::Path(_)) => m.path = Some(text()?),
                field::INTERFACE if matches!(v, Value::Str(_)) => m.interface = Some(text()?),
                field::MEMBER if matches!(v, Value::Str(_)) => m.member = Some(text()?),
                field::ERROR_NAME if matches!(v, Value::Str(_)) => m.error_name = Some(text()?),
                field::DESTINATION if matches!(v, Value::Str(_)) => m.destination = Some(text()?),
                field::SENDER if matches!(v, Value::Str(_)) => m.sender = Some(text()?),
                field::SIGNATURE if matches!(v, Value::Signature(_)) => signature = text()?,
                field::REPLY_SERIAL => m.reply_serial = Some(v.as_u32().ok_or(Error("reply serial"))?),
                field::UNIX_FDS => m.unix_fds = v.as_u32().ok_or(Error("unix fds"))?,
                field::PATH..=field::UNIX_FDS => return Err(Error("a header field of the wrong type")),
                _ => {} // Unknown fields are ignored.
            }
        }
        let required = match kind {
            MessageType::MethodCall => m.path.is_some() && m.member.is_some(),
            MessageType::Signal => m.path.is_some() && m.interface.is_some() && m.member.is_some(),
            MessageType::Error => m.error_name.is_some() && m.reply_serial.is_some(),
            MessageType::MethodReturn => m.reply_serial.is_some(),
        };
        if !required {
            return Err(Error("a required header field is missing"));
        }
        let start = r.pos.next_multiple_of(8);
        if bytes[r.pos..start].iter().any(|&b| b != 0) {
            return Err(Error("header padding is not zero"));
        }
        if start + body_len != bytes.len() {
            return Err(Error("the body length does not match"));
        }
        m.body = wire::unmarshal(&bytes[start..], &signature, big)?;
        Ok(m)
    }
}

/// Cuts a connection's byte stream into messages.
#[derive(Debug, Default)]
pub struct Stream {
    buf: Vec<u8>,
}

impl Stream {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete message (`Some(Err)`: the stream is broken).
    pub fn next_message(&mut self) -> Option<Result<Message>> {
        let len = match Message::length(&self.buf)? {
            Ok(len) => len,
            Err(e) => return Some(Err(e)),
        };
        if self.buf.len() < len {
            return None;
        }
        let rest = self.buf.split_off(len);
        let bytes = core::mem::replace(&mut self.buf, rest);
        Some(Message::decode(&bytes))
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_method_call_round_trips() {
        let mut m = Message::method_call(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "Hello",
            vec![],
        );
        let b = m.encode(1);
        assert_eq!(&b[..4], b"l\x01\x00\x01");
        assert_eq!(b.len() % 8, 0, "no body: the header padded to 8");
        assert_eq!(Message::length(&b), Some(Ok(b.len())));
        m.serial = 1;
        assert_eq!(Message::decode(&b).unwrap(), m);
        // With a body and a sender.
        let mut call = Message::method_call(None, "/a", None, "Do", vec![Value::str("x"), Value::Uint32(9)]);
        call.sender = Some(":1.4".into());
        call.flags = NO_REPLY_EXPECTED;
        let b = call.encode(77);
        let d = Message::decode(&b).unwrap();
        assert_eq!((d.serial, d.sender.as_deref(), d.signature().as_str()), (77, Some(":1.4"), "su"));
        assert!(!d.wants_reply());
        // Replies go to the caller.
        let r = Message::method_return(&d, vec![Value::Uint32(1)]);
        assert_eq!((r.reply_serial, r.destination.as_deref()), (Some(77), Some(":1.4")));
        let e = Message::error(&d, "org.example.Error", "no");
        let e = Message::decode(&e.encode(3)).unwrap();
        assert_eq!(e.error_name.as_deref(), Some("org.example.Error"));
    }

    #[test]
    fn streams_are_cut_into_messages() {
        let a = Message::signal("/a", "org.example.I", "S", vec![Value::Int32(5)]).encode(1);
        let b = Message::method_call(None, "/", None, "M", vec![]).encode(2);
        let mut s = Stream::default();
        let mut all = a.clone();
        all.extend_from_slice(&b);
        s.push(&all[..5]);
        assert!(s.next_message().is_none());
        s.push(&all[5..a.len() + 3]);
        assert_eq!(s.next_message().unwrap().unwrap().member.as_deref(), Some("S"));
        assert!(s.next_message().is_none());
        s.push(&all[a.len() + 3..]);
        assert_eq!(s.next_message().unwrap().unwrap().serial, 2);
        assert!(s.is_empty());
    }

    #[test]
    fn bad_headers() {
        let good = Message::method_call(None, "/", None, "M", vec![]).encode(1);
        let mut b = good.clone();
        b[3] = 2;
        assert!(Message::decode(&b).is_err(), "version 2");
        let mut b = good.clone();
        b[1] = 9;
        assert!(Message::decode(&b).is_err(), "type 9");
        let mut b = good.clone();
        b[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(Message::decode(&b).is_err(), "serial 0");
        // A method call without a member.
        let mut m = Message::method_call(None, "/", None, "M", vec![]);
        m.member = None;
        assert!(Message::decode(&m.encode(1)).is_err());
        assert_eq!(Message::length(b"x234567890123456"), Some(Err(Error("unknown byte order"))));
    }
}
