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

/// [`DEVICE`]'s number: a major of the range Linux's `devices.txt` keeps
/// for local and experimental use (240 to 254), minor 0.
pub const DEVICE_MAJOR: u32 = 250;
pub const DEVICE_MINOR: u32 = 0;

/// The device request that copies rows of a buffer into the compositor's
/// memory (a buffer's handle, in this first version of the link: the
/// compositor keeps its own copy of each buffer and refreshes the damaged
/// rows before it applies a [`ToCompositor::Damage`]). The argument points
/// to a [`ReadBuffer`]. Encoded as Linux's `_IOW('W', 1, 24)`
/// (`asm-generic/ioctl.h`).
pub const READ_BUFFER: u32 = (1 << 30) | ((ReadBuffer::SIZE as u32) << 16) | ((b'W' as u32) << 8) | 1;

/// [`READ_BUFFER`]'s argument: rows `row .. row + rows` of `buffer`
/// (`stride` bytes each) go to `dst`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadBuffer {
    pub buffer: BufferId,
    pub row: u32,
    pub rows: u32,
    pub dst: u64,
}

impl ReadBuffer {
    /// Its size in memory: `buffer`, `row`, `rows` (`u32` each), 4 bytes
    /// of padding, `dst` (`u64`).
    pub const SIZE: usize = 24;

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.buffer.to_le_bytes());
        b[4..8].copy_from_slice(&self.row.to_le_bytes());
        b[8..12].copy_from_slice(&self.rows.to_le_bytes());
        b[16..24].copy_from_slice(&self.dst.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> ReadBuffer {
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let mut dst = [0u8; 8];
        dst.copy_from_slice(&b[16..24]);
        ReadBuffer { buffer: u32_at(0), row: u32_at(4), rows: u32_at(8), dst: u64::from_le_bytes(dst) }
    }
}

/// A layer's buffer (ADR 0224): memory the compositor maps through [`DEVICE`] itself
/// (`map_handle` of the device's handle) at this offset, read-only and shared. The pages are the
/// app's own: a section its process draws in.
pub fn layer_buffer_offset(buffer: BufferId) -> u64 {
    u64::from(buffer) << 32
}

/// The buffer a mapping offset of [`DEVICE`] names ([`layer_buffer_offset`]), if it is one.
pub fn layer_buffer_at(offset: u64) -> Option<BufferId> {
    (offset & 0xFFFF_FFFF == 0 && offset >> 32 != 0).then_some((offset >> 32) as BufferId)
}

/// A layer of a surface: a picture another part of the app shows over the surface's own buffer
/// (a DXGI swap chain presenting into the window or into one of its child windows, ADR 0224).
pub type LayerId = u32;

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

/// The most bytes a single bridged clipboard item may carry (16 MiB).
pub const MAX_CLIPBOARD_BYTES: usize = 16 * 1024 * 1024;

/// The most payload bytes one clipboard chunk record carries (32 KiB, well
/// within the 16-bit record length).
pub const CLIPBOARD_CHUNK: usize = 32 * 1024;

/// A canonical clipboard format exchanged across the desktop link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipFormat {
    /// UTF-8 text (LF line endings, no trailing NUL).
    Utf8Text,
    /// Packed Windows `CF_DIB` (`BITMAPINFOHEADER` + optional masks/palette + pixels).
    Dib,
}

/// A complete clipboard update assembled from link records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipPayload {
    pub seq: u32,
    pub text: Option<Vec<u8>>,
    pub dib: Option<Vec<u8>>,
}

impl ClipPayload {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.dib.is_none()
    }
}

/// Reassembles chunked `ClipData` records until `ClipCommit` (or `ClipClear`).
/// Shared by both win32k and the compositor.
#[derive(Clone, Debug, Default)]
pub struct ClipAssembly {
    seq: Option<u32>,
    text: Option<(usize, Vec<u8>)>,
    dib: Option<(usize, Vec<u8>)>,
    failed: bool,
}

impl ClipAssembly {
    pub fn new() -> ClipAssembly {
        ClipAssembly::default()
    }

    pub fn reset(&mut self) {
        *self = ClipAssembly::default();
    }

    /// Record a `ClipClear`: resets any in-progress assembly and returns an
    /// empty payload immediately.
    pub fn clear(&mut self, seq: u32) -> ClipPayload {
        self.reset();
        ClipPayload { seq, text: None, dib: None }
    }

    /// Append one `ClipData` chunk for generation `seq`. A newer `seq`
    /// supersedes any incomplete earlier generation.
    pub fn push(&mut self, seq: u32, format: ClipFormat, offset: u32, total: u32, data: &[u8]) {
        if self.seq != Some(seq) {
            self.reset();
            self.seq = Some(seq);
        }
        if self.failed {
            return;
        }
        let (offset, total) = (offset as usize, total as usize);
        if total > MAX_CLIPBOARD_BYTES || offset.checked_add(data.len()).is_none_or(|end| end > total) {
            self.failed = true;
            return;
        }
        let slot = match format {
            ClipFormat::Utf8Text => &mut self.text,
            ClipFormat::Dib => &mut self.dib,
        };
        match slot {
            None => {
                if offset != 0 {
                    self.failed = true;
                    return;
                }
                let mut buf = Vec::with_capacity(total.min(CLIPBOARD_CHUNK));
                buf.extend_from_slice(data);
                *slot = Some((total, buf));
            }
            Some((expected, buf)) => {
                if *expected != total || offset != buf.len() {
                    self.failed = true;
                    return;
                }
                buf.extend_from_slice(data);
            }
        }
    }

    /// Finish generation `seq` when `ClipCommit` arrives. Returns `Some` only
    /// when `seq` matches and every started format received all its bytes.
    pub fn commit(&mut self, seq: u32) -> Option<ClipPayload> {
        if self.seq != Some(seq) || self.failed {
            self.reset();
            return None;
        }
        let finish = |slot: Option<(usize, Vec<u8>)>| -> Result<Option<Vec<u8>>, ()> {
            match slot {
                None => Ok(None),
                Some((total, buf)) if buf.len() == total => Ok(Some(buf)),
                Some(_) => Err(()),
            }
        };
        let text = finish(self.text.take());
        let dib = finish(self.dib.take());
        self.reset();
        Some(ClipPayload { seq, text: text.ok()?, dib: dib.ok()? })
    }
}

/// Append `ToCompositor` clipboard records for `(seq, text, dib)` to `out`.
pub fn encode_to_compositor_clip(out: &mut Vec<u8>, seq: u32, text: Option<&[u8]>, dib: Option<&[u8]>) {
    if text.is_none() && dib.is_none() {
        ToCompositor::ClipClear { seq }.encode(out);
        return;
    }
    for (format, payload) in [(ClipFormat::Utf8Text, text), (ClipFormat::Dib, dib)] {
        let Some(bytes) = payload else { continue };
        if bytes.len() > MAX_CLIPBOARD_BYTES {
            continue;
        }
        let total = bytes.len() as u32;
        if bytes.is_empty() {
            ToCompositor::ClipData { seq, format, offset: 0, total: 0, data: Vec::new() }.encode(out);
            continue;
        }
        for (i, chunk) in bytes.chunks(CLIPBOARD_CHUNK).enumerate() {
            ToCompositor::ClipData { seq, format, offset: (i * CLIPBOARD_CHUNK) as u32, total, data: chunk.to_vec() }
                .encode(out);
        }
    }
    ToCompositor::ClipCommit { seq }.encode(out);
}

/// Append `FromCompositor` clipboard records for `(seq, text, dib)` to `out`.
pub fn encode_from_compositor_clip(out: &mut Vec<u8>, seq: u32, text: Option<&[u8]>, dib: Option<&[u8]>) {
    if text.is_none() && dib.is_none() {
        FromCompositor::ClipClear { seq }.encode(out);
        return;
    }
    for (format, payload) in [(ClipFormat::Utf8Text, text), (ClipFormat::Dib, dib)] {
        let Some(bytes) = payload else { continue };
        if bytes.len() > MAX_CLIPBOARD_BYTES {
            continue;
        }
        let total = bytes.len() as u32;
        if bytes.is_empty() {
            FromCompositor::ClipData { seq, format, offset: 0, total: 0, data: Vec::new() }.encode(out);
            continue;
        }
        for (i, chunk) in bytes.chunks(CLIPBOARD_CHUNK).enumerate() {
            FromCompositor::ClipData { seq, format, offset: (i * CLIPBOARD_CHUNK) as u32, total, data: chunk.to_vec() }
                .encode(out);
        }
    }
    FromCompositor::ClipCommit { seq }.encode(out);
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
    /// The process that owns the surface's window (sent after `Create`, and
    /// replayed with it): the compositor ends the startup feedback of the
    /// app it launched when this process shows its first window (ADR 0167).
    Process { surface: SurfaceId, pid: u32 },
    /// The window's own non-client borders around the surface, in pixels:
    /// a window that draws its own caption (ADR 0183) keeps the sizing
    /// borders its `WM_NCCALCSIZE` answer left (invisible, as on Windows),
    /// so the compositor passes the pointer there to it as well (negative or
    /// beyond-the-surface coordinates) and its `WM_NCHITTEST` can answer
    /// `HTLEFT` and the others (ADR 0155 addendum "browsing"). All 0 for
    /// none. A compositor that does not know the record skips it.
    Margins { surface: SurfaceId, left: u16, top: u16, right: u16, bottom: u16 },
    /// A buffer of a layer of the surface (ADR 0224): `width` x `height` pixels `B, G, R, x`
    /// (`Xrgb8888`), rows `stride` bytes apart. The compositor maps it from the device at
    /// [`layer_buffer_offset`]; it stays the layer's until [`ToCompositor::LayerDrop`] or
    /// [`ToCompositor::LayerGone`].
    LayerBuffer { surface: SurfaceId, layer: LayerId, buffer: BufferId, width: u32, height: u32, stride: u32 },
    /// Where the layer is: its buffer's pixel (0, 0) is at (`x`, `y`) of the surface's buffer, and
    /// it is shown only inside the surface rectangle `clip` (x, y, width, height). Sent before the
    /// layer's first damage and whenever it changes.
    LayerPlace { surface: SurfaceId, layer: LayerId, x: i32, y: i32, clip: (u32, u32, u32, u32) },
    /// The layer shows `buffer` now, which changed in this rectangle (buffer pixels): the
    /// compositor takes it from there. The rest of the layer is as it was.
    LayerDamage { surface: SurfaceId, layer: LayerId, buffer: BufferId, x: u32, y: u32, width: u32, height: u32 },
    /// The layer's buffer `buffer` goes (the compositor unmaps it).
    LayerDrop { surface: SurfaceId, layer: LayerId, buffer: BufferId },
    /// The layer goes with its buffers; the surface's own picture shows where it was.
    LayerGone { surface: SurfaceId, layer: LayerId },
    /// A Windows program asks the desktop to open a file or folder with the program Poly's
    /// Files would use (`show` false: `ShellExecuteExW` with no association, ADR 0223), or to
    /// show it in Files, selected in its folder (`show` true: `SHOpenFolderAndSelectItems`).
    /// `path` is the object's path in Poly's file system. Not replayed. A compositor that does
    /// not know the record skips it.
    Shell { show: bool, path: String },
    /// A Windows program asks to run a program with administrator rights (ADR 0222): the
    /// compositor shows its consent prompt (the program's name, its publisher, its path, Yes and
    /// No) and answers with [`FromCompositor::Consent`] for `request`. `pid` is the asking
    /// process, `path` the Windows path, `file` the same file in Poly's file system (empty if it
    /// has none), from which the compositor reads the publisher. The texts come from the kernel
    /// (the image the program named), never from the app's windows. A compositor that does not
    /// know the record skips it, and the request is answered No when the link ends.
    Consent { request: u32, pid: u32, program: String, path: String, file: String },
    /// The Windows clipboard was cleared (or has no bridgeable materialized formats).
    ClipClear { seq: u32 },
    /// One chunk of a Windows clipboard format for generation `seq`.
    ClipData { seq: u32, format: ClipFormat, offset: u32, total: u32, data: Vec<u8> },
    /// All formats for Windows clipboard generation `seq` have been sent.
    ClipCommit { seq: u32 },
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
    /// The pointer left the surface (and its frame), for the place (x, y) relative to the
    /// surface's top left (as `Pointer`): win32k's cursor follows it there.
    Leave { surface: SurfaceId, x: i32, y: i32 },
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
    /// Windows apps live in `units` output pixels per logical pixel (ADR 0219): the sizes in
    /// `Hello` and every position and size on the link are already in those pixels, and the
    /// system DPI is 96 times `units`. Without this record: 1.
    Units { units: u32 },
    /// The user's answer to [`ToCompositor::Consent`] `request`: Yes (`allow`) or No.
    Consent { request: u32, allow: bool },
    /// The Wayland/desktop selection was cleared (or has no bridgeable formats).
    ClipClear { seq: u32 },
    /// One chunk of a Wayland/desktop selection format for generation `seq`.
    ClipData { seq: u32, format: ClipFormat, offset: u32, total: u32, data: Vec<u8> },
    /// All formats for Wayland/desktop selection generation `seq` have been sent.
    ClipCommit { seq: u32 },
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
    fn bytes_u16(&mut self) -> Result<Vec<u8>, Error> {
        let n = usize::from(self.u16()?);
        if self.bytes.len() < n {
            return Err(Error::BadLength);
        }
        let (b, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(b.to_vec())
    }
    fn clip_format(&mut self) -> Result<ClipFormat, Error> {
        match self.u8()? {
            1 => Ok(ClipFormat::Utf8Text),
            2 => Ok(ClipFormat::Dib),
            _ => Err(Error::BadValue),
        }
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
    fn bytes_u16(self, b: &[u8]) -> Self {
        let b = &b[..b.len().min(CLIPBOARD_CHUNK)];
        self.u16(b.len() as u16).bytes(b)
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
    pub const PROCESS: u16 = 0x000E;
    pub const MARGINS: u16 = 0x000F;
    pub const SHELL: u16 = 0x0010;
    pub const LAYER_BUFFER: u16 = 0x0011;
    pub const LAYER_PLACE: u16 = 0x0012;
    pub const LAYER_DAMAGE: u16 = 0x0013;
    pub const LAYER_DROP: u16 = 0x0014;
    pub const LAYER_GONE: u16 = 0x0015;
    pub const CONSENT: u16 = 0x0016;
    pub const CLIP_CLEAR: u16 = 0x0017;
    pub const CLIP_DATA: u16 = 0x0018;
    pub const CLIP_COMMIT: u16 = 0x0019;

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
    pub const UNITS: u16 = 0x800D;
    pub const CONSENT_ANSWER: u16 = 0x800E;
    pub const CLIP_CLEAR_FROM: u16 = 0x800F;
    pub const CLIP_DATA_FROM: u16 = 0x8010;
    pub const CLIP_COMMIT_FROM: u16 = 0x8011;
}

fn clip_format_of(f: ClipFormat) -> u8 {
    match f {
        ClipFormat::Utf8Text => 1,
        ClipFormat::Dib => 2,
    }
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
            Process { surface, pid } => Out::new(out, kind::PROCESS).u32(*surface).u32(*pid).done(),
            Margins { surface, left, top, right, bottom } => {
                Out::new(out, kind::MARGINS).u32(*surface).u16(*left).u16(*top).u16(*right).u16(*bottom).done()
            }
            LayerBuffer { surface, layer, buffer, width, height, stride } => Out::new(out, kind::LAYER_BUFFER)
                .u32(*surface)
                .u32(*layer)
                .u32(*buffer)
                .u32(*width)
                .u32(*height)
                .u32(*stride)
                .done(),
            LayerPlace { surface, layer, x, y, clip } => Out::new(out, kind::LAYER_PLACE)
                .u32(*surface)
                .u32(*layer)
                .i32(*x)
                .i32(*y)
                .u32(clip.0)
                .u32(clip.1)
                .u32(clip.2)
                .u32(clip.3)
                .done(),
            LayerDamage { surface, layer, buffer, x, y, width, height } => Out::new(out, kind::LAYER_DAMAGE)
                .u32(*surface)
                .u32(*layer)
                .u32(*buffer)
                .u32(*x)
                .u32(*y)
                .u32(*width)
                .u32(*height)
                .done(),
            LayerDrop { surface, layer, buffer } => {
                Out::new(out, kind::LAYER_DROP).u32(*surface).u32(*layer).u32(*buffer).done()
            }
            LayerGone { surface, layer } => Out::new(out, kind::LAYER_GONE).u32(*surface).u32(*layer).done(),
            Shell { show, path } => Out::new(out, kind::SHELL).bool(*show).string(path).done(),
            Consent { request, pid, program, path, file } => {
                Out::new(out, kind::CONSENT).u32(*request).u32(*pid).string(program).string(path).string(file).done()
            }
            ClipClear { seq } => Out::new(out, kind::CLIP_CLEAR).u32(*seq).done(),
            ClipData { seq, format, offset, total, data } => Out::new(out, kind::CLIP_DATA)
                .u32(*seq)
                .u8(clip_format_of(*format))
                .u32(*offset)
                .u32(*total)
                .bytes_u16(data)
                .done(),
            ClipCommit { seq } => Out::new(out, kind::CLIP_COMMIT).u32(*seq).done(),
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
            kind::PROCESS => Process { surface: b.u32()?, pid: b.u32()? },
            kind::MARGINS => {
                Margins { surface: b.u32()?, left: b.u16()?, top: b.u16()?, right: b.u16()?, bottom: b.u16()? }
            }
            kind::LAYER_BUFFER => LayerBuffer {
                surface: b.u32()?,
                layer: b.u32()?,
                buffer: b.u32()?,
                width: b.u32()?,
                height: b.u32()?,
                stride: b.u32()?,
            },
            kind::LAYER_PLACE => LayerPlace {
                surface: b.u32()?,
                layer: b.u32()?,
                x: b.i32()?,
                y: b.i32()?,
                clip: (b.u32()?, b.u32()?, b.u32()?, b.u32()?),
            },
            kind::LAYER_DAMAGE => LayerDamage {
                surface: b.u32()?,
                layer: b.u32()?,
                buffer: b.u32()?,
                x: b.u32()?,
                y: b.u32()?,
                width: b.u32()?,
                height: b.u32()?,
            },
            kind::LAYER_DROP => LayerDrop { surface: b.u32()?, layer: b.u32()?, buffer: b.u32()? },
            kind::LAYER_GONE => LayerGone { surface: b.u32()?, layer: b.u32()? },
            kind::SHELL => Shell { show: b.bool()?, path: b.string()? },
            kind::CONSENT => {
                Consent { request: b.u32()?, pid: b.u32()?, program: b.string()?, path: b.string()?, file: b.string()? }
            }
            kind::CLIP_CLEAR => ClipClear { seq: b.u32()? },
            kind::CLIP_DATA => ClipData {
                seq: b.u32()?,
                format: b.clip_format()?,
                offset: b.u32()?,
                total: b.u32()?,
                data: b.bytes_u16()?,
            },
            kind::CLIP_COMMIT => ClipCommit { seq: b.u32()? },
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
            Leave { surface, x, y } => Out::new(out, kind::LEAVE).u32(*surface).i32(*x).i32(*y).done(),
            Key { code, pressed, time } => Out::new(out, kind::KEY).u16(*code).bool(*pressed).u32(*time).done(),
            Focus { surface } => Out::new(out, kind::FOCUS).u32(*surface).done(),
            Configure { surface, x, y, width, height } => {
                Out::new(out, kind::CONFIGURE).u32(*surface).i32(*x).i32(*y).u32(*width).u32(*height).done()
            }
            Request { surface, request } => Out::new(out, kind::REQUEST).u32(*surface).u8(request_of(*request)).done(),
            Layout { name } => Out::new(out, kind::LAYOUT).string(name).done(),
            Released { buffer } => Out::new(out, kind::RELEASED).u32(*buffer).done(),
            Units { units } => Out::new(out, kind::UNITS).u32(*units).done(),
            Consent { request, allow } => Out::new(out, kind::CONSENT_ANSWER).u32(*request).bool(*allow).done(),
            ClipClear { seq } => Out::new(out, kind::CLIP_CLEAR_FROM).u32(*seq).done(),
            ClipData { seq, format, offset, total, data } => Out::new(out, kind::CLIP_DATA_FROM)
                .u32(*seq)
                .u8(clip_format_of(*format))
                .u32(*offset)
                .u32(*total)
                .bytes_u16(data)
                .done(),
            ClipCommit { seq } => Out::new(out, kind::CLIP_COMMIT_FROM).u32(*seq).done(),
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
            kind::LEAVE => Leave { surface: b.u32()?, x: b.i32()?, y: b.i32()? },
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
            kind::UNITS => Units { units: b.u32()? },
            kind::CONSENT_ANSWER => Consent { request: b.u32()?, allow: b.bool()? },
            kind::CLIP_CLEAR_FROM => ClipClear { seq: b.u32()? },
            kind::CLIP_DATA_FROM => ClipData {
                seq: b.u32()?,
                format: b.clip_format()?,
                offset: b.u32()?,
                total: b.u32()?,
                data: b.bytes_u16()?,
            },
            kind::CLIP_COMMIT_FROM => ClipCommit { seq: b.u32()? },
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
            Process { surface: 1, pid: 4242 },
            Margins { surface: 1, left: 8, top: 0, right: 8, bottom: 8 },
            LayerBuffer { surface: 1, layer: 3, buffer: 11, width: 640, height: 400, stride: 2560 },
            LayerPlace { surface: 1, layer: 3, x: -2, y: 30, clip: (0, 30, 638, 400) },
            LayerDamage { surface: 1, layer: 3, buffer: 11, x: 5, y: 6, width: 7, height: 8 },
            LayerDrop { surface: 1, layer: 3, buffer: 11 },
            LayerGone { surface: 1, layer: 3 },
            Consent {
                request: 7,
                pid: 4242,
                program: String::from("ChromeSetup.exe"),
                path: String::from(r"C:\Users\poly\Downloads\ChromeSetup.exe"),
                file: String::from("/win/c/Users/poly/Downloads/ChromeSetup.exe"),
            },
            ClipClear { seq: 11 },
            ClipData { seq: 12, format: ClipFormat::Utf8Text, offset: 0, total: 5, data: b"hello".to_vec() },
            ClipData { seq: 12, format: ClipFormat::Dib, offset: 0, total: 4, data: vec![1, 2, 3, 4] },
            ClipCommit { seq: 12 },
        ]
    }

    #[test]
    fn a_layer_buffer_is_mapped_at_its_own_offset() {
        assert_eq!(layer_buffer_offset(7), 7 << 32);
        assert_eq!(layer_buffer_at(layer_buffer_offset(7)), Some(7));
        assert_eq!(layer_buffer_at(0), None);
        assert_eq!(layer_buffer_at((7 << 32) | 4096), None);
    }

    fn from_samples() -> Vec<FromCompositor> {
        use FromCompositor::*;
        vec![
            Hello { version: VERSION, width: 1280, height: 800, scale: 2, frame: FrameMetrics::POLY },
            Output { width: 1920, height: 1080, scale: 1 },
            Pointer { surface: 1, x: -3, y: -12, time: 77 },
            Button { surface: 1, button: 0x110, pressed: true, time: 78 },
            Axis { surface: 1, vertical: -120, horizontal: 0, time: 79 },
            Leave { surface: 1, x: -40, y: 900 },
            Key { code: 30, pressed: false, time: 80 },
            Focus { surface: 0 },
            Configure { surface: 1, x: 0, y: 25, width: 800, height: 600 },
            Request { surface: 1, request: self::Request::Close },
            Layout { name: String::from("ch") },
            Released { buffer: 9 },
            Units { units: 2 },
            Consent { request: 7, allow: true },
            ClipClear { seq: 21 },
            ClipData { seq: 22, format: ClipFormat::Utf8Text, offset: 0, total: 3, data: b"abc".to_vec() },
            ClipData { seq: 22, format: ClipFormat::Dib, offset: 0, total: 2, data: vec![9, 8] },
            ClipCommit { seq: 22 },
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
    fn shell_records_carry_the_path() {
        let mut out = Vec::new();
        let r = ToCompositor::Shell { show: true, path: String::from("/nt/c/Users/poly/Downloads/a b.zip") };
        r.encode(&mut out);
        let mut s = Stream::<ToCompositor>::new();
        s.push(&out);
        assert_eq!(s.next_record(), Some(Ok(r)));
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

    #[test]
    fn clipboard_chunking_and_assembly_round_trip() {
        // Multi-chunk (> CLIPBOARD_CHUNK) text + image in both directions.
        let big_text: Vec<u8> = (0..(CLIPBOARD_CHUNK * 2 + 17)).map(|i| b'a' + (i % 26) as u8).collect();
        let big_dib: Vec<u8> = (0..(CLIPBOARD_CHUNK + 5)).map(|i| (i & 0xFF) as u8).collect();
        let mut wire = Vec::new();
        encode_to_compositor_clip(&mut wire, 42, Some(&big_text), Some(&big_dib));
        let mut stream = Stream::<ToCompositor>::new();
        stream.push(&wire);
        let mut asm = ClipAssembly::new();
        let mut assembled = None;
        for rec in stream.map(Result::unwrap) {
            match rec {
                ToCompositor::ClipClear { seq } => assembled = Some(asm.clear(seq)),
                ToCompositor::ClipData { seq, format, offset, total, data } => {
                    asm.push(seq, format, offset, total, &data);
                }
                ToCompositor::ClipCommit { seq } => assembled = asm.commit(seq),
                _ => panic!("unexpected record"),
            }
        }
        assert_eq!(assembled, Some(ClipPayload { seq: 42, text: Some(big_text), dib: Some(big_dib) }));

        // Empty string is distinct from ClipClear.
        let mut empty_str_wire = Vec::new();
        encode_from_compositor_clip(&mut empty_str_wire, 5, Some(b""), None);
        let mut s = Stream::<FromCompositor>::new();
        s.push(&empty_str_wire);
        let mut asm = ClipAssembly::new();
        let mut out = None;
        for rec in s.map(Result::unwrap) {
            match rec {
                FromCompositor::ClipClear { seq } => out = Some(asm.clear(seq)),
                FromCompositor::ClipData { seq, format, offset, total, data } => {
                    asm.push(seq, format, offset, total, &data);
                }
                FromCompositor::ClipCommit { seq } => out = asm.commit(seq),
                _ => panic!(),
            }
        }
        assert_eq!(out, Some(ClipPayload { seq: 5, text: Some(Vec::new()), dib: None }));

        // Clear produces an empty payload.
        let mut clear_wire = Vec::new();
        encode_from_compositor_clip(&mut clear_wire, 6, None, None);
        let mut s = Stream::<FromCompositor>::new();
        s.push(&clear_wire);
        assert_eq!(s.next_record(), Some(Ok(FromCompositor::ClipClear { seq: 6 })));

        // Newer seq supersedes an incomplete earlier transfer; oversized total rejected.
        let mut asm = ClipAssembly::new();
        asm.push(1, ClipFormat::Utf8Text, 0, 100, b"partial");
        asm.push(2, ClipFormat::Utf8Text, 0, 2, b"ok");
        assert_eq!(asm.commit(2), Some(ClipPayload { seq: 2, text: Some(b"ok".to_vec()), dib: None }));
        asm.push(3, ClipFormat::Dib, 0, (MAX_CLIPBOARD_BYTES + 1) as u32, b"x");
        assert_eq!(asm.commit(3), None);
    }
}
