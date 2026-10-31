//! The Wayland wire format ("The Wayland Protocol", chapter "Wire Format").
//!
//! A message is a header of two 32-bit words in the host's byte order, the
//! sender's object ID, then the message size in bytes (header included) in
//! the upper 16 bits and the opcode in the lower 16, followed by its
//! arguments, each a whole number of 32-bit words:
//!
//! | Type | On the wire |
//! |---|---|
//! | `int`, `uint` | one word |
//! | `fixed` | one word, signed 24.8 fixed point |
//! | `object` | one word, an object ID; 0 is null where `allow-null` |
//! | `new_id` | one word, the new object's ID; without an `interface` in the XML, a `string` interface name and a `uint` version first |
//! | `string` | a `uint` length counting the terminating NUL, the bytes, the NUL, padding to a word; length 0 is a null string |
//! | `array` | a `uint` byte length, the bytes, padding to a word |
//! | `fd` | nothing: the file descriptor travels out of band (`SCM_RIGHTS`), in order |
//!
//! [`decode_args`] turns a message body into [`Arg`]s given its
//! [`MessageDesc`] (the generated tables in [`crate::protocol`]), taking
//! file descriptors from a queue in order; [`encode`] does the reverse.
//! Nothing here panics on any input.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Bytes in a message header.
pub const HEADER_SIZE: usize = 8;

/// The largest message the header can describe (its size field is 16 bits).
pub const MAX_MESSAGE_SIZE: usize = 0xffff;

/// The first object ID the server allocates; clients use `1..SERVER_ID_START`.
pub const SERVER_ID_START: u32 = 0xff00_0000;

/// An object ID. `wl_display` is always 1; 0 is no object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u32);

impl ObjectId {
    /// The `wl_display` singleton.
    pub const DISPLAY: ObjectId = ObjectId(1);

    /// Whether the server allocated this ID.
    pub const fn is_server(self) -> bool {
        self.0 >= SERVER_ID_START
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A signed 24.8 fixed-point number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fixed(pub i32);

impl Fixed {
    pub const fn from_int(v: i32) -> Fixed {
        Fixed(v.wrapping_mul(256))
    }

    /// The integer part, rounded towards negative infinity.
    pub const fn floor(self) -> i32 {
        self.0 >> 8
    }

    pub fn from_f64(v: f64) -> Fixed {
        Fixed((v * 256.0) as i32)
    }

    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / 256.0
    }
}

/// A file descriptor number, received from or to be sent to a client.
/// Whoever holds one is responsible for closing it (the server hands those
/// it does not keep to [`crate::server::Host::close_fd`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fd(pub i32);

/// The type of an argument, as the protocol XML names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    Int,
    Uint,
    Fixed,
    String,
    Object,
    NewId,
    Array,
    Fd,
}

/// One argument of a message.
#[derive(Clone, Copy, Debug)]
pub struct ArgDesc {
    pub name: &'static str,
    pub kind: ArgKind,
    /// The interface of an `object` or `new_id`; `None` for any (a
    /// `new_id` without one carries its interface name and version).
    pub interface: Option<&'static str>,
    /// `allow-null="true"`: an `object` may be 0, a `string` absent.
    pub nullable: bool,
}

/// A request or event.
#[derive(Clone, Copy, Debug)]
pub struct MessageDesc {
    pub name: &'static str,
    /// The interface version that added it.
    pub since: u32,
    /// `type="destructor"`: the object is gone after it.
    pub destructor: bool,
    pub args: &'static [ArgDesc],
}

/// An interface: its requests and events, indexed by opcode.
#[derive(Debug)]
pub struct Interface {
    pub name: &'static str,
    pub version: u32,
    pub requests: &'static [MessageDesc],
    pub events: &'static [MessageDesc],
}

/// The interface, version and ID a `new_id` without a fixed interface
/// carries (`wl_registry.bind`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewObject {
    pub interface: String,
    pub version: u32,
    pub id: ObjectId,
}

/// A typed request or event of some interface (the generated `Request` and
/// `Event` enums), as far as sending it goes.
pub trait Message {
    fn opcode(&self) -> u16;
    fn into_args(self) -> Vec<Arg>;
}

/// A decoded argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i32),
    Uint(u32),
    Fixed(Fixed),
    Str(Option<String>),
    Object(Option<ObjectId>),
    NewId(ObjectId),
    /// A `new_id` without an interface in the XML.
    DynNewId(NewObject),
    Array(Vec<u8>),
    Fd(Fd),
}

/// A message header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub object: ObjectId,
    pub opcode: u16,
    /// The whole message in bytes, header included.
    pub size: u16,
}

impl Header {
    /// The header at the start of `bytes`, if there are 8 bytes.
    pub fn parse(bytes: &[u8]) -> Option<Header> {
        let object = word(bytes, 0)?;
        let second = word(bytes, 4)?;
        Some(Header { object: ObjectId(object), opcode: second as u16, size: (second >> 16) as u16 })
    }

    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut out = [0; HEADER_SIZE];
        out[..4].copy_from_slice(&self.object.0.to_ne_bytes());
        out[4..].copy_from_slice(&(u32::from(self.size) << 16 | u32::from(self.opcode)).to_ne_bytes());
        out
    }
}

/// What is wrong with a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The header's size is smaller than a header or not a multiple of 4.
    BadSize(u16),
    /// The body ends inside an argument.
    Truncated,
    /// Bytes are left after the last argument.
    TrailingBytes,
    /// An `fd` argument, but no file descriptor came with the message.
    MissingFd,
    /// A string without its terminating NUL, or with a NUL inside.
    BadString,
    /// A null `object`, `string` or `new_id` where the XML does not allow one.
    Null(&'static str),
    /// No such request or event.
    BadOpcode(u16),
    /// The arguments do not match the message (a typed conversion found
    /// another kind, or too few).
    ArgMismatch,
    /// The message would be longer than [`MAX_MESSAGE_SIZE`].
    TooLong,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::BadSize(s) => write!(f, "bad message size {s}"),
            WireError::Truncated => f.write_str("message truncated"),
            WireError::TrailingBytes => f.write_str("bytes after the last argument"),
            WireError::MissingFd => f.write_str("missing file descriptor"),
            WireError::BadString => f.write_str("malformed string"),
            WireError::Null(arg) => write!(f, "null {arg}"),
            WireError::BadOpcode(op) => write!(f, "no opcode {op}"),
            WireError::ArgMismatch => f.write_str("arguments do not match the message"),
            WireError::TooLong => f.write_str("message too long"),
        }
    }
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
}

const fn padded(len: usize) -> usize {
    (len + 3) & !3
}

/// A cursor over a message body.
struct Reader<'a> {
    body: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn word(&mut self) -> Result<u32, WireError> {
        let w = word(self.body, self.at).ok_or(WireError::Truncated)?;
        self.at += 4;
        Ok(w)
    }

    /// `len` bytes and the padding after them.
    fn bytes(&mut self, len: usize) -> Result<&[u8], WireError> {
        let end = self.at.checked_add(len).ok_or(WireError::Truncated)?;
        let bytes = self.body.get(self.at..end).ok_or(WireError::Truncated)?;
        let next = self.at.checked_add(padded(len)).ok_or(WireError::Truncated)?;
        if next > self.body.len() {
            return Err(WireError::Truncated);
        }
        self.at = next;
        Ok(bytes)
    }

    fn string(&mut self) -> Result<Option<String>, WireError> {
        let len = self.word()? as usize;
        if len == 0 {
            return Ok(None);
        }
        let bytes = self.bytes(len)?;
        let (last, text) = bytes.split_last().ok_or(WireError::BadString)?;
        if *last != 0 || text.contains(&0) {
            return Err(WireError::BadString);
        }
        // Strings are UTF-8; a client that breaks that gets replacement
        // characters rather than a disconnect.
        Ok(Some(String::from_utf8_lossy(text).into_owned()))
    }
}

/// Decode the arguments of the message `desc` from `body` (the bytes after
/// the header), taking file descriptors from `fds` in order.
pub fn decode_args(
    desc: &MessageDesc,
    body: &[u8],
    fds: &mut dyn FnMut() -> Option<Fd>,
) -> Result<Vec<Arg>, WireError> {
    let mut r = Reader { body, at: 0 };
    let mut out = Vec::with_capacity(desc.args.len());
    for a in desc.args {
        let arg = match a.kind {
            ArgKind::Int => Arg::Int(r.word()? as i32),
            ArgKind::Uint => Arg::Uint(r.word()?),
            ArgKind::Fixed => Arg::Fixed(Fixed(r.word()? as i32)),
            ArgKind::String => {
                let s = r.string()?;
                if s.is_none() && !a.nullable {
                    return Err(WireError::Null(a.name));
                }
                Arg::Str(s)
            }
            ArgKind::Object => {
                let id = r.word()?;
                if id == 0 && !a.nullable {
                    return Err(WireError::Null(a.name));
                }
                Arg::Object((id != 0).then_some(ObjectId(id)))
            }
            ArgKind::NewId if a.interface.is_none() => {
                let interface = r.string()?.ok_or(WireError::Null(a.name))?;
                let version = r.word()?;
                let id = r.word()?;
                if id == 0 {
                    return Err(WireError::Null(a.name));
                }
                Arg::DynNewId(NewObject { interface, version, id: ObjectId(id) })
            }
            ArgKind::NewId => {
                let id = r.word()?;
                if id == 0 {
                    return Err(WireError::Null(a.name));
                }
                Arg::NewId(ObjectId(id))
            }
            ArgKind::Array => {
                let len = r.word()? as usize;
                Arg::Array(r.bytes(len)?.to_vec())
            }
            ArgKind::Fd => Arg::Fd(fds().ok_or(WireError::MissingFd)?),
        };
        out.push(arg);
    }
    if r.at != body.len() {
        return Err(WireError::TrailingBytes);
    }
    Ok(out)
}

/// Append the message `opcode` from `object` with `args` to `out`, and its
/// file descriptors to `fds`.
pub fn encode(
    out: &mut Vec<u8>,
    fds: &mut Vec<Fd>,
    object: ObjectId,
    opcode: u16,
    args: &[Arg],
) -> Result<(), WireError> {
    let start = out.len();
    out.extend_from_slice(&[0; HEADER_SIZE]);
    let fds_before = fds.len();
    let put_bytes = |out: &mut Vec<u8>, bytes: &[u8], len: u32| {
        out.extend_from_slice(&len.to_ne_bytes());
        out.extend_from_slice(bytes);
        out.resize(out.len() + padded(bytes.len()) - bytes.len(), 0);
    };
    let put_string = |out: &mut Vec<u8>, s: Option<&str>| match s {
        None => out.extend_from_slice(&0u32.to_ne_bytes()),
        Some(s) => {
            let len = s.len() as u32 + 1;
            out.extend_from_slice(&len.to_ne_bytes());
            out.extend_from_slice(s.as_bytes());
            out.push(0);
            out.resize(out.len() + padded(s.len() + 1) - (s.len() + 1), 0);
        }
    };
    for arg in args {
        match arg {
            Arg::Int(v) => out.extend_from_slice(&v.to_ne_bytes()),
            Arg::Uint(v) => out.extend_from_slice(&v.to_ne_bytes()),
            Arg::Fixed(v) => out.extend_from_slice(&v.0.to_ne_bytes()),
            Arg::Str(s) => put_string(out, s.as_deref()),
            Arg::Object(o) => out.extend_from_slice(&o.map_or(0, |o| o.0).to_ne_bytes()),
            Arg::NewId(id) => out.extend_from_slice(&id.0.to_ne_bytes()),
            Arg::DynNewId(n) => {
                put_string(out, Some(&n.interface));
                out.extend_from_slice(&n.version.to_ne_bytes());
                out.extend_from_slice(&n.id.0.to_ne_bytes());
            }
            Arg::Array(a) => put_bytes(out, a, a.len() as u32),
            Arg::Fd(fd) => fds.push(*fd),
        }
    }
    let size = out.len() - start;
    if size > MAX_MESSAGE_SIZE {
        out.truncate(start);
        fds.truncate(fds_before);
        return Err(WireError::TooLong);
    }
    let header = Header { object, opcode, size: size as u16 };
    out[start..start + HEADER_SIZE].copy_from_slice(&header.encode());
    Ok(())
}

/// The arguments of a message, taken one at a time by the generated
/// conversions in [`crate::protocol`].
#[derive(Debug)]
pub struct Args(alloc::vec::IntoIter<Arg>);

impl Args {
    pub fn new(args: Vec<Arg>) -> Args {
        Args(args.into_iter())
    }

    fn next(&mut self) -> Result<Arg, WireError> {
        self.0.next().ok_or(WireError::ArgMismatch)
    }

    pub fn int(&mut self) -> Result<i32, WireError> {
        match self.next()? {
            Arg::Int(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    pub fn uint(&mut self) -> Result<u32, WireError> {
        match self.next()? {
            Arg::Uint(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    pub fn fixed(&mut self) -> Result<Fixed, WireError> {
        match self.next()? {
            Arg::Fixed(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    pub fn string(&mut self) -> Result<String, WireError> {
        self.string_opt()?.ok_or(WireError::Null("string"))
    }

    pub fn string_opt(&mut self) -> Result<Option<String>, WireError> {
        match self.next()? {
            Arg::Str(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    pub fn object(&mut self) -> Result<ObjectId, WireError> {
        self.object_opt()?.ok_or(WireError::Null("object"))
    }

    pub fn object_opt(&mut self) -> Result<Option<ObjectId>, WireError> {
        match self.next()? {
            Arg::Object(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    pub fn new_id(&mut self) -> Result<ObjectId, WireError> {
        match self.next()? {
            Arg::NewId(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    pub fn dyn_new_id(&mut self) -> Result<NewObject, WireError> {
        match self.next()? {
            Arg::DynNewId(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    pub fn array(&mut self) -> Result<Vec<u8>, WireError> {
        match self.next()? {
            Arg::Array(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    pub fn fd(&mut self) -> Result<Fd, WireError> {
        match self.next()? {
            Arg::Fd(v) => Ok(v),
            _ => Err(WireError::ArgMismatch),
        }
    }

    /// Fails if arguments are left over.
    pub fn end(mut self) -> Result<(), WireError> {
        if self.0.next().is_some() { Err(WireError::ArgMismatch) } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests;
