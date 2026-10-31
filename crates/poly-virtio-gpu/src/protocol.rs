//! The control queue's wire format (VIRTIO 1.2 §5.7.6.7, §5.7.6.8).
//!
//! Every request starts with a 24-byte `virtio_gpu_ctrl_hdr` and goes to
//! the device in one device-readable buffer; the device writes its response,
//! which starts with the same header, into one device-writable buffer. All
//! fields are little-endian.
//!
//! ```text
//! struct virtio_gpu_ctrl_hdr { le32 type; le32 flags; le64 fence_id; le32 ctx_id; u8 ring_idx; u8 padding[3]; };
//! struct virtio_gpu_rect     { le32 x; le32 y; le32 width; le32 height; };
//! ```
//!
//! [`Command::encode`] writes a request; [`decode_response`] checks a
//! response (its type, the fence, its length) and parses what it carries.

/// Request and response types (`enum virtio_gpu_ctrl_type`, §5.7.6.7).
pub mod ctrl {
    // 2D commands.
    pub const GET_DISPLAY_INFO: u32 = 0x0100;
    pub const RESOURCE_CREATE_2D: u32 = 0x0101;
    pub const RESOURCE_UNREF: u32 = 0x0102;
    pub const SET_SCANOUT: u32 = 0x0103;
    pub const RESOURCE_FLUSH: u32 = 0x0104;
    pub const TRANSFER_TO_HOST_2D: u32 = 0x0105;
    pub const RESOURCE_ATTACH_BACKING: u32 = 0x0106;
    pub const RESOURCE_DETACH_BACKING: u32 = 0x0107;
    pub const GET_CAPSET_INFO: u32 = 0x0108;
    pub const GET_CAPSET: u32 = 0x0109;
    pub const GET_EDID: u32 = 0x010a;
    pub const RESOURCE_ASSIGN_UUID: u32 = 0x010b;
    pub const RESOURCE_CREATE_BLOB: u32 = 0x010c;
    pub const SET_SCANOUT_BLOB: u32 = 0x010d;
    // Cursor commands (cursor queue).
    pub const UPDATE_CURSOR: u32 = 0x0300;
    pub const MOVE_CURSOR: u32 = 0x0301;
    // Success responses.
    pub const RESP_OK_NODATA: u32 = 0x1100;
    pub const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
    pub const RESP_OK_CAPSET_INFO: u32 = 0x1102;
    pub const RESP_OK_CAPSET: u32 = 0x1103;
    pub const RESP_OK_EDID: u32 = 0x1104;
    pub const RESP_OK_RESOURCE_UUID: u32 = 0x1105;
    pub const RESP_OK_MAP_INFO: u32 = 0x1106;
    // Error responses.
    pub const RESP_ERR_UNSPEC: u32 = 0x1200;
    pub const RESP_ERR_OUT_OF_MEMORY: u32 = 0x1201;
    pub const RESP_ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
    pub const RESP_ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
    pub const RESP_ERR_INVALID_CONTEXT_ID: u32 = 0x1204;
    pub const RESP_ERR_INVALID_PARAMETER: u32 = 0x1205;
}

/// The name of a request or response type, for messages.
pub const fn ctrl_name(ty: u32) -> &'static str {
    match ty {
        ctrl::GET_DISPLAY_INFO => "GET_DISPLAY_INFO",
        ctrl::RESOURCE_CREATE_2D => "RESOURCE_CREATE_2D",
        ctrl::RESOURCE_UNREF => "RESOURCE_UNREF",
        ctrl::SET_SCANOUT => "SET_SCANOUT",
        ctrl::RESOURCE_FLUSH => "RESOURCE_FLUSH",
        ctrl::TRANSFER_TO_HOST_2D => "TRANSFER_TO_HOST_2D",
        ctrl::RESOURCE_ATTACH_BACKING => "RESOURCE_ATTACH_BACKING",
        ctrl::RESOURCE_DETACH_BACKING => "RESOURCE_DETACH_BACKING",
        ctrl::GET_EDID => "GET_EDID",
        ctrl::RESP_OK_NODATA => "OK_NODATA",
        ctrl::RESP_OK_DISPLAY_INFO => "OK_DISPLAY_INFO",
        ctrl::RESP_OK_EDID => "OK_EDID",
        ctrl::RESP_ERR_UNSPEC => "ERR_UNSPEC",
        ctrl::RESP_ERR_OUT_OF_MEMORY => "ERR_OUT_OF_MEMORY",
        ctrl::RESP_ERR_INVALID_SCANOUT_ID => "ERR_INVALID_SCANOUT_ID",
        ctrl::RESP_ERR_INVALID_RESOURCE_ID => "ERR_INVALID_RESOURCE_ID",
        ctrl::RESP_ERR_INVALID_CONTEXT_ID => "ERR_INVALID_CONTEXT_ID",
        ctrl::RESP_ERR_INVALID_PARAMETER => "ERR_INVALID_PARAMETER",
        _ => "unknown",
    }
}

/// `virtio_gpu_ctrl_hdr.flags` (§5.7.6.7).
pub mod flags {
    /// The device answers only once the command is complete, echoing the
    /// flag and `fence_id` (§5.7.6.5).
    pub const FENCE: u32 = 1 << 0;
    /// `ring_idx` is valid (context init only).
    pub const INFO_RING_IDX: u32 = 1 << 1;
}

/// Resource formats (`enum virtio_gpu_formats`, §5.7.6.8). The name gives
/// the byte order in memory: B8G8R8X8 is blue, green, red, unused, which
/// is `poly_gfx::PixelFormat::Xrgb8888` (a little-endian `0x00RRGGBB`).
pub mod format {
    pub const B8G8R8A8_UNORM: u32 = 1;
    pub const B8G8R8X8_UNORM: u32 = 2;
    pub const A8R8G8B8_UNORM: u32 = 3;
    pub const X8R8G8B8_UNORM: u32 = 4;
    pub const R8G8B8A8_UNORM: u32 = 67;
    pub const X8B8G8R8_UNORM: u32 = 68;
    pub const A8B8G8R8_UNORM: u32 = 121;
    pub const R8G8B8X8_UNORM: u32 = 134;
}

/// Bytes per pixel of every format above.
pub const BYTES_PER_PIXEL: u32 = 4;

/// `VIRTIO_GPU_MAX_SCANOUTS` (§5.7.6.8).
pub const MAX_SCANOUTS: usize = 16;
/// `struct virtio_gpu_ctrl_hdr`.
pub const HEADER_LEN: usize = 24;
/// `struct virtio_gpu_rect`.
pub const RECT_LEN: usize = 16;
/// `struct virtio_gpu_display_one`: a rectangle, `enabled`, `flags`.
pub const DISPLAY_ONE_LEN: usize = RECT_LEN + 8;
/// `struct virtio_gpu_resp_display_info`.
pub const DISPLAY_INFO_LEN: usize = HEADER_LEN + MAX_SCANOUTS * DISPLAY_ONE_LEN;
/// Most bytes of EDID in a `virtio_gpu_resp_edid`.
pub const EDID_MAX: usize = 1024;
/// `struct virtio_gpu_resp_edid`: header, `size`, padding, the blob.
pub const EDID_RESP_LEN: usize = HEADER_LEN + 8 + EDID_MAX;
/// `struct virtio_gpu_mem_entry`: `le64 addr; le32 length; le32 padding`.
pub const MEM_ENTRY_LEN: usize = 16;
/// `struct virtio_gpu_resource_attach_backing` without its entries.
pub const ATTACH_BACKING_LEN: usize = HEADER_LEN + 8;
/// The longest response this driver asks for.
pub const RESPONSE_MAX: usize = EDID_RESP_LEN;

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn get32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn get64(b: &[u8], at: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(v)
}

/// `struct virtio_gpu_rect`: a rectangle in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0, y: 0, width: 0, height: 0 };

    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    /// `width` × `height` at the origin.
    pub const fn sized(width: u32, height: u32) -> Rect {
        Rect { x: 0, y: 0, width, height }
    }

    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// One past the right edge, without overflow.
    pub const fn right(&self) -> u64 {
        self.x as u64 + self.width as u64
    }

    /// One past the bottom edge, without overflow.
    pub const fn bottom(&self) -> u64 {
        self.y as u64 + self.height as u64
    }

    /// Whether the rectangle lies within a `width` × `height` resource.
    pub const fn fits(&self, width: u32, height: u32) -> bool {
        self.right() <= width as u64 && self.bottom() <= height as u64
    }

    /// The part of `self` inside a `width` × `height` resource (empty if
    /// none).
    pub fn clip(&self, width: u32, height: u32) -> Rect {
        let x = self.x.min(width);
        let y = self.y.min(height);
        let r = self.right().min(width as u64) as u32;
        let b = self.bottom().min(height as u64) as u32;
        Rect { x, y, width: r - x, height: b - y }
    }

    pub fn encode(&self, b: &mut [u8]) {
        put32(b, 0, self.x);
        put32(b, 4, self.y);
        put32(b, 8, self.width);
        put32(b, 12, self.height);
    }

    pub fn decode(b: &[u8]) -> Rect {
        Rect { x: get32(b, 0), y: get32(b, 4), width: get32(b, 8), height: get32(b, 12) }
    }
}

/// `struct virtio_gpu_ctrl_hdr`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CtrlHeader {
    pub ty: u32,
    pub flags: u32,
    pub fence_id: u64,
    pub ctx_id: u32,
    pub ring_idx: u8,
}

impl CtrlHeader {
    pub fn encode(&self, b: &mut [u8]) {
        put32(b, 0, self.ty);
        put32(b, 4, self.flags);
        put64(b, 8, self.fence_id);
        put32(b, 16, self.ctx_id);
        b[20] = self.ring_idx;
        b[21..24].fill(0);
    }

    pub fn decode(b: &[u8]) -> CtrlHeader {
        CtrlHeader { ty: get32(b, 0), flags: get32(b, 4), fence_id: get64(b, 8), ctx_id: get32(b, 16), ring_idx: b[20] }
    }
}

/// `struct virtio_gpu_mem_entry`: a piece of guest memory backing a
/// resource, at a bus address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemEntry {
    pub addr: u64,
    pub length: u32,
}

/// A control-queue request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command<'a> {
    /// The scanouts' preferred modes and whether each is enabled.
    GetDisplayInfo,
    /// A scanout's EDID (with VIRTIO_GPU_F_EDID).
    GetEdid { scanout: u32 },
    /// A host-side 2D resource (an image) of `width` × `height` in
    /// `format`, called `resource_id` (not 0) from then on.
    ResourceCreate2d { resource_id: u32, format: u32, width: u32, height: u32 },
    /// Destroy a resource.
    ResourceUnref { resource_id: u32 },
    /// Show `rect` of a resource on a scanout; resource 0 turns the
    /// scanout off.
    SetScanout { scanout_id: u32, resource_id: u32, rect: Rect },
    /// Show what changed in `rect` of a resource on the scanouts showing it.
    ResourceFlush { resource_id: u32, rect: Rect },
    /// Copy `rect` from the resource's guest memory to the host resource;
    /// `offset` is where the rectangle's first pixel lies in that memory.
    TransferToHost2d { resource_id: u32, rect: Rect, offset: u64 },
    /// Give a resource its guest memory (a scatter list).
    ResourceAttachBacking { resource_id: u32, entries: &'a [MemEntry] },
    /// Take a resource's guest memory back.
    ResourceDetachBacking { resource_id: u32 },
}

impl Command<'_> {
    /// The request type.
    pub const fn ty(&self) -> u32 {
        match self {
            Command::GetDisplayInfo => ctrl::GET_DISPLAY_INFO,
            Command::GetEdid { .. } => ctrl::GET_EDID,
            Command::ResourceCreate2d { .. } => ctrl::RESOURCE_CREATE_2D,
            Command::ResourceUnref { .. } => ctrl::RESOURCE_UNREF,
            Command::SetScanout { .. } => ctrl::SET_SCANOUT,
            Command::ResourceFlush { .. } => ctrl::RESOURCE_FLUSH,
            Command::TransferToHost2d { .. } => ctrl::TRANSFER_TO_HOST_2D,
            Command::ResourceAttachBacking { .. } => ctrl::RESOURCE_ATTACH_BACKING,
            Command::ResourceDetachBacking { .. } => ctrl::RESOURCE_DETACH_BACKING,
        }
    }

    /// Bytes of the request.
    pub const fn len(&self) -> usize {
        HEADER_LEN
            + match self {
                Command::GetDisplayInfo => 0,
                // le32 scanout; le32 padding.
                Command::GetEdid { .. } => 8,
                // le32 resource_id; le32 format; le32 width; le32 height.
                Command::ResourceCreate2d { .. } => 16,
                // le32 resource_id; le32 padding.
                Command::ResourceUnref { .. } | Command::ResourceDetachBacking { .. } => 8,
                // rect; le32 scanout_id; le32 resource_id.
                Command::SetScanout { .. } => RECT_LEN + 8,
                // rect; le32 resource_id; le32 padding.
                Command::ResourceFlush { .. } => RECT_LEN + 8,
                // rect; le64 offset; le32 resource_id; le32 padding.
                Command::TransferToHost2d { .. } => RECT_LEN + 16,
                // le32 resource_id; le32 nr_entries; then the entries.
                Command::ResourceAttachBacking { entries, .. } => 8 + entries.len() * MEM_ENTRY_LEN,
            }
    }

    /// Never empty: every request has at least its header.
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// The response type that means success.
    pub const fn ok_type(&self) -> u32 {
        match self {
            Command::GetDisplayInfo => ctrl::RESP_OK_DISPLAY_INFO,
            Command::GetEdid { .. } => ctrl::RESP_OK_EDID,
            _ => ctrl::RESP_OK_NODATA,
        }
    }

    /// Bytes of the successful response.
    pub const fn response_len(&self) -> usize {
        match self {
            Command::GetDisplayInfo => DISPLAY_INFO_LEN,
            Command::GetEdid { .. } => EDID_RESP_LEN,
            _ => HEADER_LEN,
        }
    }

    /// Write the request into `b`, fenced with `fence` if given. Returns
    /// its length, or `None` if `b` is too short.
    pub fn encode(&self, fence: Option<u64>, b: &mut [u8]) -> Option<usize> {
        let len = self.len();
        let b = b.get_mut(..len)?;
        b.fill(0);
        let header = CtrlHeader {
            ty: self.ty(),
            flags: if fence.is_some() { flags::FENCE } else { 0 },
            fence_id: fence.unwrap_or(0),
            ctx_id: 0,
            ring_idx: 0,
        };
        header.encode(b);
        let body = &mut b[HEADER_LEN..];
        match *self {
            Command::GetDisplayInfo => {}
            Command::GetEdid { scanout } => put32(body, 0, scanout),
            Command::ResourceCreate2d { resource_id, format, width, height } => {
                put32(body, 0, resource_id);
                put32(body, 4, format);
                put32(body, 8, width);
                put32(body, 12, height);
            }
            Command::ResourceUnref { resource_id } | Command::ResourceDetachBacking { resource_id } => {
                put32(body, 0, resource_id)
            }
            Command::SetScanout { scanout_id, resource_id, rect } => {
                rect.encode(body);
                put32(body, RECT_LEN, scanout_id);
                put32(body, RECT_LEN + 4, resource_id);
            }
            Command::ResourceFlush { resource_id, rect } => {
                rect.encode(body);
                put32(body, RECT_LEN, resource_id);
            }
            Command::TransferToHost2d { resource_id, rect, offset } => {
                rect.encode(body);
                put64(body, RECT_LEN, offset);
                put32(body, RECT_LEN + 8, resource_id);
            }
            Command::ResourceAttachBacking { resource_id, entries } => {
                put32(body, 0, resource_id);
                put32(body, 4, entries.len() as u32);
                for (i, e) in entries.iter().enumerate() {
                    let at = 8 + i * MEM_ENTRY_LEN;
                    put64(body, at, e.addr);
                    put32(body, at + 8, e.length);
                }
            }
        }
        Some(len)
    }
}

/// One scanout of `struct virtio_gpu_resp_display_info`: its preferred
/// position and size, whether it is enabled (connected), and flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanoutInfo {
    pub rect: Rect,
    pub enabled: bool,
    pub flags: u32,
}

/// What `GET_DISPLAY_INFO` returned, one entry per possible scanout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayInfo {
    pub scanouts: [ScanoutInfo; MAX_SCANOUTS],
}

impl DisplayInfo {
    /// Parse the body (after the header) of a display info response.
    pub fn decode(body: &[u8]) -> DisplayInfo {
        let mut d = DisplayInfo::default();
        for (i, s) in d.scanouts.iter_mut().enumerate() {
            let at = i * DISPLAY_ONE_LEN;
            let one = &body[at..at + DISPLAY_ONE_LEN];
            *s = ScanoutInfo {
                rect: Rect::decode(one),
                enabled: get32(one, RECT_LEN) != 0,
                flags: get32(one, RECT_LEN + 4),
            };
        }
        d
    }

    /// The enabled scanouts with a usable size among the first `count`,
    /// with their numbers.
    pub fn enabled(&self, count: u32) -> impl Iterator<Item = (u32, ScanoutInfo)> + '_ {
        let n = (count as usize).min(MAX_SCANOUTS);
        self.scanouts[..n]
            .iter()
            .enumerate()
            .filter(|(_, s)| s.enabled && !s.rect.is_empty())
            .map(|(i, s)| (i as u32, *s))
    }
}

/// An EDID blob from `GET_EDID`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EdidBlob {
    pub len: usize,
    pub data: [u8; EDID_MAX],
}

impl EdidBlob {
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

impl core::fmt::Debug for EdidBlob {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "EdidBlob({} bytes)", self.len)
    }
}

/// A successful response.
///
/// The EDID variant carries its kilobyte inline: the crate allocates
/// nothing, and a response lives on the stack only until its command's
/// method returns what it carries.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    NoData,
    DisplayInfo(DisplayInfo),
    Edid(EdidBlob),
}

/// What was wrong with a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseError {
    /// The device reported an error (`RESP_ERR_*`, or an unexpected type).
    Device { request: u32, response: u32 },
    /// Shorter than the header, or than the response type needs.
    TooShort { len: usize },
    /// A fenced request whose response lacks the fence or has another ID
    /// (§5.7.6.5).
    Fence { want: u64, got: Option<u64> },
    /// An EDID response claiming more than [`EDID_MAX`] bytes.
    BadEdidSize(u32),
}

/// Check the response the device wrote (`written` bytes of `b`) to
/// `cmd`, sent with `fence`, and parse it.
pub fn decode_response(
    cmd: &Command<'_>,
    fence: Option<u64>,
    b: &[u8],
    written: usize,
) -> Result<Response, ResponseError> {
    let written = written.min(b.len());
    if written < HEADER_LEN {
        return Err(ResponseError::TooShort { len: written });
    }
    let h = CtrlHeader::decode(b);
    if h.ty != cmd.ok_type() {
        return Err(ResponseError::Device { request: cmd.ty(), response: h.ty });
    }
    if let Some(want) = fence {
        let got = (h.flags & flags::FENCE != 0).then_some(h.fence_id);
        if got != Some(want) {
            return Err(ResponseError::Fence { want, got });
        }
    }
    // §2.7.8: nothing past the first `written` bytes may be relied on.
    let min = match h.ty {
        ctrl::RESP_OK_DISPLAY_INFO => DISPLAY_INFO_LEN,
        ctrl::RESP_OK_EDID => HEADER_LEN + 8,
        _ => HEADER_LEN,
    };
    if written < min {
        return Err(ResponseError::TooShort { len: written });
    }
    match h.ty {
        ctrl::RESP_OK_DISPLAY_INFO => Ok(Response::DisplayInfo(DisplayInfo::decode(&b[HEADER_LEN..]))),
        ctrl::RESP_OK_EDID => {
            let size = get32(b, HEADER_LEN);
            if size as usize > EDID_MAX {
                return Err(ResponseError::BadEdidSize(size));
            }
            let len = size as usize;
            // The blob must be there, as far as the device says it is.
            if written < HEADER_LEN + 8 + len {
                return Err(ResponseError::TooShort { len: written });
            }
            let mut data = [0u8; EDID_MAX];
            data[..len].copy_from_slice(&b[HEADER_LEN + 8..HEADER_LEN + 8 + len]);
            Ok(Response::Edid(EdidBlob { len, data }))
        }
        _ => Ok(Response::NoData),
    }
}
