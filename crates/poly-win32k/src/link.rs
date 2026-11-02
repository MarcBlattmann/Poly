//! The desktop link: Poly's own protocol between win32k (the NT
//! personality, in the kernel) and the native compositor (ADR 0128,
//! Decision 3).
//!
//! The compositor is the one process that opens [`DEVICE`]; it writes
//! [`FromCompositor`] records and reads [`ToCompositor`] records. A record
//! is little-endian: `kind: u16`, `length: u16` (the whole record, header
//! included), then its fields in the order of the variant. Strings are a
//! `u16` byte count and UTF-8. The compositor starts with
//! [`FromCompositor::Hello`]; win32k answers [`ToCompositor::Welcome`] and
//! replays every shown surface ([`crate::desktop`]). A reader skips a
//! record of a kind it does not know (its length says how far), so either
//! side may grow new records under a higher [`VERSION`].
//!
//! Coordinates are logical pixels of the output (the compositor's scale
//! applies on top); a surface's pixels are described by its
//! [`ToCompositor::Buffer`] record. Times are milliseconds of the
//! compositor's monotonic clock, as Wayland's input events carry them.

use crate::frame::FrameMetrics;
use alloc::string::String;
use alloc::vec::Vec;

/// The link's version, sent in both directions of the greeting.
pub const VERSION: u32 = 1;

/// The device the compositor opens (one opener at a time).
pub const DEVICE: &str = "/dev/win32k";

/// The longest record (the length field is 16 bits).
pub const MAX_RECORD: usize = u16::MAX as usize;

/// The longest string a record carries; longer titles are cut at a
/// character boundary.
pub const MAX_TEXT: usize = 4096;

/// win32k's name for a surface (one per shown top-level window); never 0.
pub type SurfaceId = u32;
/// win32k's name for a buffer of a surface; never 0.
pub type BufferId = u32;

/// What kind of top-level window a surface shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    /// An application window: in the shell's window list, activatable.
    TopLevel,
    /// A menu, tooltip, drop-down: never in the window list, kept above its
    /// owner, never framed.
    Popup,
}

/// How a buffer's 32-bit pixels are read (`B, G, R, A` in memory, a 32-bpp
/// top-down DIB's layout, read as little-endian `0xAARRGGBB`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// The alpha byte is ignored: the surface is opaque.
    Xrgb8888,
    /// Premultiplied alpha (layered windows; Learn, `BLENDFUNCTION`).
    Argb8888,
}

/// A window's state as the app set it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Maximized,
    Minimized,
}

/// What the shell asks of a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Close,
    Minimize,
    Maximize,
    Restore,
}

/// Which edges an interactive resize moves (bits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges(pub u8);

impl Edges {
    pub const LEFT: Edges = Edges(1);
    pub const TOP: Edges = Edges(2);
    pub const RIGHT: Edges = Edges(4);
    pub const BOTTOM: Edges = Edges(8);

    pub const fn union(self, other: Edges) -> Edges {
        Edges(self.0 | other.0)
    }

    pub const fn contains(self, other: Edges) -> bool {
        self.0 & other.0 == other.0
    }
}

/// The cursor over a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// A standard cursor: the `IDC_*` resource number of `winuser.h`
    /// (`IDC_ARROW` is 32512).
    Standard(u16),
    /// An image: an `Argb8888` buffer of the surface and its hot spot.
    Image { buffer: BufferId, hot_x: u16, hot_y: u16 },
}

/// win32k → compositor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToCompositor {
    /// The answer to [`FromCompositor::Hello`]; the replay follows.
    Welcome { version: u32 },
    /// A top-level window got a surface. `owner` (0: none) is kept below it.
    Create { surface: SurfaceId, kind: SurfaceKind, owner: SurfaceId, framed: bool },
    /// The surface's rectangle on the screen: the window rectangle less
    /// Poly's frame ([`crate::frame`]).
    Geometry { surface: SurfaceId, x: i32, y: i32, width: u32, height: u32 },
    /// The window text.
    Title { surface: SurfaceId, text: String },
    /// Shown or hidden.
    Visible { surface: SurfaceId, shown: bool },
    /// A new buffer for the surface; the compositor takes its handle with a
    /// device request naming `buffer`, and maps it.
    Buffer { surface: SurfaceId, buffer: BufferId, width: u32, height: u32, stride: u32, format: PixelFormat },
    /// Pixels of the current buffer changed (buffer coordinates).
    Damage { surface: SurfaceId, x: u32, y: u32, width: u32, height: u32 },
    /// The app asks to be the foreground window.
    Activate { surface: SurfaceId },
    /// The cursor to show over the surface.
    Cursor { surface: SurfaceId, shape: CursorShape },
    /// `DefWindowProc` started an interactive move.
    BeginMove { surface: SurfaceId },
    /// `DefWindowProc` started an interactive resize of `edges`.
    BeginResize { surface: SurfaceId, edges: Edges },
    /// The app maximized, minimized or restored the window.
    State { surface: SurfaceId, state: WindowState },
    /// The surface is gone (its buffers with it).
    Destroy { surface: SurfaceId },
}

/// compositor → win32k.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromCompositor {
    /// The greeting: the output's logical size and scale, Poly's frame.
    Hello { version: u32, width: u32, height: u32, scale: u32, frame: FrameMetrics },
    /// The output changed.
    Output { width: u32, height: u32, scale: u32 },
    /// The pointer over a surface, relative to its top left (outside the
    /// surface: over Poly's frame).
    Pointer { surface: SurfaceId, x: i32, y: i32, time: u32 },
    /// A button (evdev code: `BTN_LEFT` is 0x110).
    Button { surface: SurfaceId, button: u16, pressed: bool, time: u32 },
    /// The wheel, in `WHEEL_DELTA` (120, `winuser.h`) units per notch.
    Axis { surface: SurfaceId, vertical: i32, horizontal: i32, time: u32 },
    /// The pointer left the surface (and its frame).
    Leave { surface: SurfaceId },
    /// A key (evdev code), for the focused surface.
    Key { code: u16, pressed: bool, time: u32 },
    /// The compositor made this surface the active one (0: none of win32k's).
    Focus { surface: SurfaceId },
    /// The compositor moved or resized the surface.
    Configure { surface: SurfaceId, x: i32, y: i32, width: u32, height: u32 },
    /// The shell asks something of the window.
    Request { surface: SurfaceId, request: Request },
    /// The keyboard layout of the shell's settings (`poly-input`'s names).
    Layout { name: String },
    /// The compositor reads that buffer no more.
    Released { buffer: BufferId },
}

/// Why bytes are not a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A record of a kind this side does not know (skipped).
    UnknownKind(u16),
    /// A length below the header, or a body too short for its kind.
    BadLength,
    /// A field out of its range (an enum value, a boolean, UTF-8).
    BadValue,
}

/// A record of one direction.
pub trait Record: Sized {
    /// Append the record's wire form to `out`.
    fn encode(&self, out: &mut Vec<u8>);
    /// Read a body of `kind`.
    fn decode_body(kind: u16, body: &mut Body<'_>) -> Result<Self, Error>;
}

/// The fields of one record, read in order.
#[derive(Debug)]
pub struct Body<'a> {
    bytes: &'a [u8],
}

impl Body<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let (head, rest) = self.bytes.split_first_chunk::<N>().ok_or(Error::BadLength)?;
        self.bytes = rest;
        Ok(*head)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.take()?))
    }
    fn bool(&mut self) -> Result<bool, Error> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::BadValue),
        }
    }
    fn string(&mut self) -> Result<String, Error> {
        let n = usize::from(self.u16()?);
        if self.bytes.len() < n {
            return Err(Error::BadLength);
        }
        let (s, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        core::str::from_utf8(s).map(String::from).map_err(|_| Error::BadValue)
    }
}

/// Builds one record: the header is patched when it is finished.
struct Out<'a> {
    out: &'a mut Vec<u8>,
    start: usize,
}

impl<'a> Out<'a> {
    fn new(out: &'a mut Vec<u8>, kind: u16) -> Out<'a> {
        let start = out.len();
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        Out { out, start }
    }
    fn u8(self, v: u8) -> Self {
        self.out.push(v);
        self
    }
    fn u16(self, v: u16) -> Self {
        self.out.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(self, v: u32) -> Self {
        self.out.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(self, v: i32) -> Self {
        self.out.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bool(self, v: bool) -> Self {
        self.u8(u8::from(v))
    }
    fn string(self, s: &str) -> Self {
        let s = cut(s, MAX_TEXT);
        self.u16(s.len() as u16).bytes(s.as_bytes())
    }
    fn bytes(self, b: &[u8]) -> Self {
        self.out.extend_from_slice(b);
        self
    }
    fn done(self) {
        let len = (self.out.len() - self.start) as u16;
        self.out[self.start + 2..self.start + 4].copy_from_slice(&len.to_le_bytes());
    }
}

/// `s` cut to at most `max` bytes at a character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// Record kinds: win32k → compositor from 0x0001, compositor → win32k from
// 0x8001. Never reused.
mod kind {
    pub const WELCOME: u16 = 0x0001;
    pub const CREATE: u16 = 0x0002;
    pub const GEOMETRY: u16 = 0x0003;
    pub const TITLE: u16 = 0x0004;
    pub const VISIBLE: u16 = 0x0005;
    pub const BUFFER: u16 = 0x0006;
    pub const DAMAGE: u16 = 0x0007;
    pub const ACTIVATE: u16 = 0x0008;
    pub const CURSOR: u16 = 0x0009;
    pub const BEGIN_MOVE: u16 = 0x000A;
    pub const BEGIN_RESIZE: u16 = 0x000B;
    pub const STATE: u16 = 0x000C;
    pub const DESTROY: u16 = 0x000D;

    pub const HELLO: u16 = 0x8001;
    pub const OUTPUT: u16 = 0x8002;
    pub const POINTER: u16 = 0x8003;
    pub const BUTTON: u16 = 0x8004;
    pub const AXIS: u16 = 0x8005;
    pub const LEAVE: u16 = 0x8006;
    pub const KEY: u16 = 0x8007;
    pub const FOCUS: u16 = 0x8008;
    pub const CONFIGURE: u16 = 0x8009;
    pub const REQUEST: u16 = 0x800A;
    pub const LAYOUT: u16 = 0x800B;
    pub const RELEASED: u16 = 0x800C;
}

fn kind_of(k: SurfaceKind) -> u8 {
    match k {
        SurfaceKind::TopLevel => 0,
        SurfaceKind::Popup => 1,
    }
}

fn format_of(f: PixelFormat) -> u8 {
    match f {
        PixelFormat::Xrgb8888 => 0,
        PixelFormat::Argb8888 => 1,
    }
}

fn state_of(s: WindowState) -> u8 {
    match s {
        WindowState::Normal => 0,
        WindowState::Maximized => 1,
        WindowState::Minimized => 2,
    }
}

fn request_of(r: Request) -> u8 {
    match r {
        Request::Close => 0,
        Request::Minimize => 1,
        Request::Maximize => 2,
        Request::Restore => 3,
    }
}

impl Record for ToCompositor {
    fn encode(&self, out: &mut Vec<u8>) {
        use ToCompositor::*;
        match self {
            Welcome { version } => Out::new(out, kind::WELCOME).u32(*version).done(),
            Create { surface, kind: k, owner, framed } => {
                Out::new(out, kind::CREATE).u32(*surface).u8(kind_of(*k)).u32(*owner).bool(*framed).done()
            }
            Geometry { surface, x, y, width, height } => {
                Out::new(out, kind::GEOMETRY).u32(*surface).i32(*x).i32(*y).u32(*width).u32(*height).done()
            }
            Title { surface, text } => Out::new(out, kind::TITLE).u32(*surface).string(text).done(),
            Visible { surface, shown } => Out::new(out, kind::VISIBLE).u32(*surface).bool(*shown).done(),
            Buffer { surface, buffer, width, height, stride, format } => Out::new(out, kind::BUFFER)
                .u32(*surface)
                .u32(*buffer)
                .u32(*width)
                .u32(*height)
                .u32(*stride)
                .u8(format_of(*format))
                .done(),
            Damage { surface, x, y, width, height } => {
                Out::new(out, kind::DAMAGE).u32(*surface).u32(*x).u32(*y).u32(*width).u32(*height).done()
            }
            Activate { surface } => Out::new(out, kind::ACTIVATE).u32(*surface).done(),
            Cursor { surface, shape } => {
                let o = Out::new(out, kind::CURSOR).u32(*surface);
                match *shape {
                    CursorShape::Standard(id) => o.u8(0).u16(id).done(),
                    CursorShape::Image { buffer, hot_x, hot_y } => o.u8(1).u32(buffer).u16(hot_x).u16(hot_y).done(),
                }
            }
            BeginMove { surface } => Out::new(out, kind::BEGIN_MOVE).u32(*surface).done(),
            BeginResize { surface, edges } => Out::new(out, kind::BEGIN_RESIZE).u32(*surface).u8(edges.0).done(),
            State { surface, state } => Out::new(out, kind::STATE).u32(*surface).u8(state_of(*state)).done(),
            Destroy { surface } => Out::new(out, kind::DESTROY).u32(*surface).done(),
        }
    }

    fn decode_body(k: u16, b: &mut Body<'_>) -> Result<Self, Error> {
        use ToCompositor::*;
        Ok(match k {
            kind::WELCOME => Welcome { version: b.u32()? },
            kind::CREATE => Create {
                surface: b.u32()?,
                kind: match b.u8()? {
                    0 => SurfaceKind::TopLevel,
                    1 => SurfaceKind::Popup,
                    _ => return Err(Error::BadValue),
                },
                owner: b.u32()?,
                framed: b.bool()?,
            },
            kind::GEOMETRY => {
                Geometry { surface: b.u32()?, x: b.i32()?, y: b.i32()?, width: b.u32()?, height: b.u32()? }
            }
            kind::TITLE => Title { surface: b.u32()?, text: b.string()? },
            kind::VISIBLE => Visible { surface: b.u32()?, shown: b.bool()? },
            kind::BUFFER => Buffer {
                surface: b.u32()?,
                buffer: b.u32()?,
                width: b.u32()?,
                height: b.u32()?,
                stride: b.u32()?,
                format: match b.u8()? {
                    0 => PixelFormat::Xrgb8888,
                    1 => PixelFormat::Argb8888,
                    _ => return Err(Error::BadValue),
                },
            },
            kind::DAMAGE => Damage { surface: b.u32()?, x: b.u32()?, y: b.u32()?, width: b.u32()?, height: b.u32()? },
            kind::ACTIVATE => Activate { surface: b.u32()? },
            kind::CURSOR => {
                let surface = b.u32()?;
                let shape = match b.u8()? {
                    0 => CursorShape::Standard(b.u16()?),
                    1 => CursorShape::Image { buffer: b.u32()?, hot_x: b.u16()?, hot_y: b.u16()? },
                    _ => return Err(Error::BadValue),
                };
                Cursor { surface, shape }
            }
            kind::BEGIN_MOVE => BeginMove { surface: b.u32()? },
            kind::BEGIN_RESIZE => BeginResize { surface: b.u32()?, edges: Edges(b.u8()?) },
            kind::STATE => State {
                surface: b.u32()?,
                state: match b.u8()? {
                    0 => WindowState::Normal,
                    1 => WindowState::Maximized,
                    2 => WindowState::Minimized,
                    _ => return Err(Error::BadValue),
                },
            },
            kind::DESTROY => Destroy { surface: b.u32()? },
            other => return Err(Error::UnknownKind(other)),
        })
    }
}

impl Record for FromCompositor {
    fn encode(&self, out: &mut Vec<u8>) {
        use FromCompositor::*;
        match self {
            Hello { version, width, height, scale, frame } => Out::new(out, kind::HELLO)
                .u32(*version)
                .u32(*width)
                .u32(*height)
                .u32(*scale)
                .u16(frame.title)
                .u16(frame.border)
                .u16(frame.resize)
                .done(),
            Output { width, height, scale } => Out::new(out, kind::OUTPUT).u32(*width).u32(*height).u32(*scale).done(),
            Pointer { surface, x, y, time } => {
                Out::new(out, kind::POINTER).u32(*surface).i32(*x).i32(*y).u32(*time).done()
            }
            Button { surface, button, pressed, time } => {
                Out::new(out, kind::BUTTON).u32(*surface).u16(*button).bool(*pressed).u32(*time).done()
            }
            Axis { surface, vertical, horizontal, time } => {
                Out::new(out, kind::AXIS).u32(*surface).i32(*vertical).i32(*horizontal).u32(*time).done()
            }
            Leave { surface } => Out::new(out, kind::LEAVE).u32(*surface).done(),
            Key { code, pressed, time } => Out::new(out, kind::KEY).u16(*code).bool(*pressed).u32(*time).done(),
            Focus { surface } => Out::new(out, kind::FOCUS).u32(*surface).done(),
            Configure { surface, x, y, width, height } => {
                Out::new(out, kind::CONFIGURE).u32(*surface).i32(*x).i32(*y).u32(*width).u32(*height).done()
            }
            Request { surface, request } => Out::new(out, kind::REQUEST).u32(*surface).u8(request_of(*request)).done(),
            Layout { name } => Out::new(out, kind::LAYOUT).string(name).done(),
            Released { buffer } => Out::new(out, kind::RELEASED).u32(*buffer).done(),
        }
    }

    fn decode_body(k: u16, b: &mut Body<'_>) -> Result<Self, Error> {
        use FromCompositor::*;
        Ok(match k {
            kind::HELLO => Hello {
                version: b.u32()?,
                width: b.u32()?,
                height: b.u32()?,
                scale: b.u32()?,
                frame: FrameMetrics { title: b.u16()?, border: b.u16()?, resize: b.u16()? },
            },
            kind::OUTPUT => Output { width: b.u32()?, height: b.u32()?, scale: b.u32()? },
            kind::POINTER => Pointer { surface: b.u32()?, x: b.i32()?, y: b.i32()?, time: b.u32()? },
            kind::BUTTON => Button { surface: b.u32()?, button: b.u16()?, pressed: b.bool()?, time: b.u32()? },
            kind::AXIS => Axis { surface: b.u32()?, vertical: b.i32()?, horizontal: b.i32()?, time: b.u32()? },
            kind::LEAVE => Leave { surface: b.u32()? },
            kind::KEY => Key { code: b.u16()?, pressed: b.bool()?, time: b.u32()? },
            kind::FOCUS => Focus { surface: b.u32()? },
            kind::CONFIGURE => {
                Configure { surface: b.u32()?, x: b.i32()?, y: b.i32()?, width: b.u32()?, height: b.u32()? }
            }
            kind::REQUEST => Request {
                surface: b.u32()?,
                request: match b.u8()? {
                    0 => self::Request::Close,
                    1 => self::Request::Minimize,
                    2 => self::Request::Maximize,
                    3 => self::Request::Restore,
                    _ => return Err(Error::BadValue),
                },
            },
            kind::LAYOUT => Layout { name: b.string()? },
            kind::RELEASED => Released { buffer: b.u32()? },
            other => return Err(Error::UnknownKind(other)),
        })
    }
}

/// Encode `records` one after another.
pub fn encode_all<R: Record>(records: &[R]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in records {
        r.encode(&mut out);
    }
    out
}

/// Records out of a byte stream that may arrive in pieces (a `read` of the
/// device returns whatever fits).
#[derive(Debug)]
pub struct Stream<R> {
    buf: Vec<u8>,
    _r: core::marker::PhantomData<R>,
}

impl<R> Default for Stream<R> {
    fn default() -> Self {
        Stream { buf: Vec::new(), _r: core::marker::PhantomData }
    }
}

impl<R: Record> Stream<R> {
    pub fn new() -> Stream<R> {
        Stream::default()
    }

    /// Bytes that arrived.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Bytes received but not yet a whole record.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// The next whole record, if one has arrived. A record of an unknown
    /// kind or with bad fields is consumed and reported; a length below the
    /// header cannot be skipped, so the stream drops what it holds (the
    /// connection is broken) and reports [`Error::BadLength`].
    pub fn next_record(&mut self) -> Option<Result<R, Error>> {
        let header = self.buf.first_chunk::<4>()?;
        let k = u16::from_le_bytes([header[0], header[1]]);
        let len = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if len < 4 {
            self.buf.clear();
            return Some(Err(Error::BadLength));
        }
        if self.buf.len() < len {
            return None;
        }
        let mut body = Body { bytes: &self.buf[4..len] };
        let r = R::decode_body(k, &mut body);
        let r = match r {
            Ok(_) if !body.bytes.is_empty() => Err(Error::BadLength),
            other => other,
        };
        self.buf.drain(..len);
        Some(r)
    }
}

impl<R: Record> Iterator for Stream<R> {
    type Item = Result<R, Error>;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_record()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::vec;

    fn to_samples() -> Vec<ToCompositor> {
        use ToCompositor::*;
        vec![
            Welcome { version: VERSION },
            Create { surface: 1, kind: SurfaceKind::TopLevel, owner: 0, framed: true },
            Create { surface: 2, kind: SurfaceKind::Popup, owner: 1, framed: false },
            Geometry { surface: 1, x: -20, y: 40, width: 640, height: 480 },
            Title { surface: 1, text: String::from("Unbenannt – Editor") },
            Visible { surface: 1, shown: true },
            Buffer { surface: 1, buffer: 9, width: 640, height: 480, stride: 2560, format: PixelFormat::Xrgb8888 },
            Damage { surface: 1, x: 3, y: 4, width: 5, height: 6 },
            Activate { surface: 1 },
            Cursor { surface: 1, shape: CursorShape::Standard(32512) },
            Cursor { surface: 1, shape: CursorShape::Image { buffer: 10, hot_x: 1, hot_y: 2 } },
            BeginMove { surface: 1 },
            BeginResize { surface: 1, edges: Edges::LEFT.union(Edges::BOTTOM) },
            State { surface: 1, state: WindowState::Maximized },
            Destroy { surface: 2 },
        ]
    }

    fn from_samples() -> Vec<FromCompositor> {
        use FromCompositor::*;
        vec![
            Hello { version: VERSION, width: 1280, height: 800, scale: 2, frame: FrameMetrics::POLY },
            Output { width: 1920, height: 1080, scale: 1 },
            Pointer { surface: 1, x: -3, y: -12, time: 77 },
            Button { surface: 1, button: 0x110, pressed: true, time: 78 },
            Axis { surface: 1, vertical: -120, horizontal: 0, time: 79 },
            Leave { surface: 1 },
            Key { code: 30, pressed: false, time: 80 },
            Focus { surface: 0 },
            Configure { surface: 1, x: 0, y: 25, width: 800, height: 600 },
            Request { surface: 1, request: self::Request::Close },
            Layout { name: String::from("ch") },
            Released { buffer: 9 },
        ]
    }

    #[test]
    fn records_round_trip_in_both_directions() {
        let to = to_samples();
        let mut s = Stream::<ToCompositor>::new();
        s.push(&encode_all(&to));
        let back: Vec<_> = s.by_ref().map(Result::unwrap).collect();
        assert_eq!(back, to);
        assert_eq!(s.pending(), 0);

        let from = from_samples();
        let mut s = Stream::<FromCompositor>::new();
        s.push(&encode_all(&from));
        assert_eq!(s.map(Result::unwrap).collect::<Vec<_>>(), from);
    }

    #[test]
    fn a_stream_waits_for_whole_records() {
        let bytes = encode_all(&from_samples());
        let mut s = Stream::<FromCompositor>::new();
        let mut got = Vec::new();
        for b in &bytes {
            s.push(core::slice::from_ref(b));
            while let Some(r) = s.next_record() {
                got.push(r.unwrap());
            }
        }
        assert_eq!(got, from_samples());
    }

    #[test]
    fn the_header_carries_kind_and_length() {
        let mut out = Vec::new();
        ToCompositor::Destroy { surface: 0x0102_0304 }.encode(&mut out);
        assert_eq!(out, [0x0D, 0x00, 8, 0, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn unknown_kinds_are_skipped_and_bad_records_reported() {
        let mut bytes = vec![0x7F, 0x00, 6, 0, 0xAA, 0xBB];
        ToCompositor::Activate { surface: 5 }.encode(&mut bytes);
        // A boolean that is neither 0 nor 1.
        bytes.extend_from_slice(&[0x05, 0x00, 9, 0, 1, 0, 0, 0, 2]);
        // A body longer than its kind.
        bytes.extend_from_slice(&[0x08, 0x00, 9, 0, 1, 0, 0, 0, 0]);
        let mut s = Stream::<ToCompositor>::new();
        s.push(&bytes);
        assert_eq!(s.next_record(), Some(Err(Error::UnknownKind(0x7F))));
        assert_eq!(s.next_record(), Some(Ok(ToCompositor::Activate { surface: 5 })));
        assert_eq!(s.next_record(), Some(Err(Error::BadValue)));
        assert_eq!(s.next_record(), Some(Err(Error::BadLength)));
        assert_eq!(s.next_record(), None);
        // A length below the header breaks the stream.
        s.push(&[0x01, 0x00, 2, 0, 9, 9, 9]);
        assert_eq!(s.next_record(), Some(Err(Error::BadLength)));
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn long_titles_are_cut_at_a_character() {
        let text: String = core::iter::repeat_n('ü', MAX_TEXT).collect();
        let mut out = Vec::new();
        ToCompositor::Title { surface: 1, text }.encode(&mut out);
        let mut s = Stream::<ToCompositor>::new();
        s.push(&out);
        let Some(Ok(ToCompositor::Title { text, .. })) = s.next_record() else { panic!() };
        assert_eq!(text.len(), MAX_TEXT);
        assert!(text.chars().all(|c| c == 'ü'));
    }
}
