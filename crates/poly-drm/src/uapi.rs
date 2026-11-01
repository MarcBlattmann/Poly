//! The DRM user API: ioctl numbers, structure layouts and constants, from
//! the MIT-licensed public headers `drm/drm.h`, `drm/drm_mode.h` and
//! `drm/drm_fourcc.h` (x86-64: `long`, `size_t` and pointers are 8 bytes,
//! `__u64` is 8-byte aligned). Every structure is read from and written to
//! the ioctl's argument bytes at the header's field offsets, little-endian.

/// The ioctl type of every DRM request (`DRM_IOCTL_BASE`, `'d'`).
pub const IOCTL_BASE: u8 = b'd';

/// `_IOC` direction bits (asm-generic/ioctl.h): the caller writes the
/// argument (`_IOW`), reads it back (`_IOR`), or both.
pub mod dir {
    pub const NONE: u32 = 0;
    pub const WRITE: u32 = 1;
    pub const READ: u32 = 2;
}

/// A decoded ioctl request number (asm-generic/ioctl.h: number bits 0-7,
/// type 8-15, size 16-29, direction 30-31).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub nr: u8,
    pub ty: u8,
    pub size: usize,
    pub dir: u32,
}

impl Request {
    pub const fn decode(request: u32) -> Request {
        Request {
            nr: request as u8,
            ty: (request >> 8) as u8,
            size: ((request >> 16) & 0x3fff) as usize,
            dir: request >> 30,
        }
    }

    /// `_IOC(dir, 'd', nr, size)`.
    pub const fn encode(dir: u32, nr: u8, size: usize) -> u32 {
        (dir << 30) | ((size as u32) << 16) | ((IOCTL_BASE as u32) << 8) | nr as u32
    }
}

/// The request numbers' `nr` field (drm.h `DRM_IOCTL_*`). Each comment is
/// the header's `DRM_IO*` macro and argument type.
pub mod nr {
    /// `DRM_IOWR(0x00, struct drm_version)`
    pub const VERSION: u8 = 0x00;
    /// `DRM_IOWR(0x01, struct drm_unique)`
    pub const GET_UNIQUE: u8 = 0x01;
    /// `DRM_IOR(0x02, struct drm_auth)`
    pub const GET_MAGIC: u8 = 0x02;
    /// `DRM_IOWR(0x07, struct drm_set_version)`
    pub const SET_VERSION: u8 = 0x07;
    /// `DRM_IOW(0x08, struct drm_modeset_ctl)`
    pub const MODESET_CTL: u8 = 0x08;
    /// `DRM_IOW(0x09, struct drm_gem_close)`
    pub const GEM_CLOSE: u8 = 0x09;
    /// `DRM_IOWR(0x0a, struct drm_gem_flink)`
    pub const GEM_FLINK: u8 = 0x0a;
    /// `DRM_IOWR(0x0b, struct drm_gem_open)`
    pub const GEM_OPEN: u8 = 0x0b;
    /// `DRM_IOWR(0x0c, struct drm_get_cap)`
    pub const GET_CAP: u8 = 0x0c;
    /// `DRM_IOW(0x0d, struct drm_set_client_cap)`
    pub const SET_CLIENT_CAP: u8 = 0x0d;
    /// `DRM_IOW(0x11, struct drm_auth)`
    pub const AUTH_MAGIC: u8 = 0x11;
    /// `DRM_IO(0x1e)`
    pub const SET_MASTER: u8 = 0x1e;
    /// `DRM_IO(0x1f)`
    pub const DROP_MASTER: u8 = 0x1f;
    /// `DRM_IOWR(0x2d, struct drm_prime_handle)`
    pub const PRIME_HANDLE_TO_FD: u8 = 0x2d;
    /// `DRM_IOWR(0x2e, struct drm_prime_handle)`
    pub const PRIME_FD_TO_HANDLE: u8 = 0x2e;
    /// `DRM_IOWR(0x3a, union drm_wait_vblank)`
    pub const WAIT_VBLANK: u8 = 0x3a;
    /// `DRM_IOWR(0x3b, struct drm_crtc_get_sequence)`
    pub const CRTC_GET_SEQUENCE: u8 = 0x3b;
    /// `DRM_IOWR(0x3c, struct drm_crtc_queue_sequence)`
    pub const CRTC_QUEUE_SEQUENCE: u8 = 0x3c;
    /// `DRM_IOWR(0xA0, struct drm_mode_card_res)`
    pub const MODE_GETRESOURCES: u8 = 0xa0;
    /// `DRM_IOWR(0xA1, struct drm_mode_crtc)`
    pub const MODE_GETCRTC: u8 = 0xa1;
    /// `DRM_IOWR(0xA2, struct drm_mode_crtc)`
    pub const MODE_SETCRTC: u8 = 0xa2;
    /// `DRM_IOWR(0xA3, struct drm_mode_cursor)`
    pub const MODE_CURSOR: u8 = 0xa3;
    /// `DRM_IOWR(0xA4, struct drm_mode_crtc_lut)`
    pub const MODE_GETGAMMA: u8 = 0xa4;
    /// `DRM_IOWR(0xA5, struct drm_mode_crtc_lut)`
    pub const MODE_SETGAMMA: u8 = 0xa5;
    /// `DRM_IOWR(0xA6, struct drm_mode_get_encoder)`
    pub const MODE_GETENCODER: u8 = 0xa6;
    /// `DRM_IOWR(0xA7, struct drm_mode_get_connector)`
    pub const MODE_GETCONNECTOR: u8 = 0xa7;
    /// `DRM_IOWR(0xAA, struct drm_mode_get_property)`
    pub const MODE_GETPROPERTY: u8 = 0xaa;
    /// `DRM_IOWR(0xAB, struct drm_mode_connector_set_property)`
    pub const MODE_SETPROPERTY: u8 = 0xab;
    /// `DRM_IOWR(0xAC, struct drm_mode_get_blob)`
    pub const MODE_GETPROPBLOB: u8 = 0xac;
    /// `DRM_IOWR(0xAD, struct drm_mode_fb_cmd)`
    pub const MODE_GETFB: u8 = 0xad;
    /// `DRM_IOWR(0xAE, struct drm_mode_fb_cmd)`
    pub const MODE_ADDFB: u8 = 0xae;
    /// `DRM_IOWR(0xAF, unsigned int)`
    pub const MODE_RMFB: u8 = 0xaf;
    /// `DRM_IOWR(0xB0, struct drm_mode_crtc_page_flip)`
    pub const MODE_PAGE_FLIP: u8 = 0xb0;
    /// `DRM_IOWR(0xB1, struct drm_mode_fb_dirty_cmd)`
    pub const MODE_DIRTYFB: u8 = 0xb1;
    /// `DRM_IOWR(0xB2, struct drm_mode_create_dumb)`
    pub const MODE_CREATE_DUMB: u8 = 0xb2;
    /// `DRM_IOWR(0xB3, struct drm_mode_map_dumb)`
    pub const MODE_MAP_DUMB: u8 = 0xb3;
    /// `DRM_IOWR(0xB4, struct drm_mode_destroy_dumb)`
    pub const MODE_DESTROY_DUMB: u8 = 0xb4;
    /// `DRM_IOWR(0xB5, struct drm_mode_get_plane_res)`
    pub const MODE_GETPLANERESOURCES: u8 = 0xb5;
    /// `DRM_IOWR(0xB6, struct drm_mode_get_plane)`
    pub const MODE_GETPLANE: u8 = 0xb6;
    /// `DRM_IOWR(0xB7, struct drm_mode_set_plane)`
    pub const MODE_SETPLANE: u8 = 0xb7;
    /// `DRM_IOWR(0xB8, struct drm_mode_fb_cmd2)`
    pub const MODE_ADDFB2: u8 = 0xb8;
    /// `DRM_IOWR(0xB9, struct drm_mode_obj_get_properties)`
    pub const MODE_OBJ_GETPROPERTIES: u8 = 0xb9;
    /// `DRM_IOWR(0xBA, struct drm_mode_obj_set_property)`
    pub const MODE_OBJ_SETPROPERTY: u8 = 0xba;
    /// `DRM_IOWR(0xBB, struct drm_mode_cursor2)`
    pub const MODE_CURSOR2: u8 = 0xbb;
    /// `DRM_IOWR(0xBC, struct drm_mode_atomic)`
    pub const MODE_ATOMIC: u8 = 0xbc;
    /// `DRM_IOWR(0xBD, struct drm_mode_create_blob)`
    pub const MODE_CREATEPROPBLOB: u8 = 0xbd;
    /// `DRM_IOWR(0xBE, struct drm_mode_destroy_blob)`
    pub const MODE_DESTROYPROPBLOB: u8 = 0xbe;
    /// `DRM_IOWR(0xCE, struct drm_mode_fb_cmd2)`
    pub const MODE_GETFB2: u8 = 0xce;
}

/// `DRM_IOCTL_*` request numbers as userspace passes them, for tests and
/// for the C test program's cross-check.
pub mod ioctl {
    use super::{Request, dir, nr};
    const fn iowr(n: u8, size: usize) -> u32 {
        Request::encode(dir::READ | dir::WRITE, n, size)
    }
    const fn iow(n: u8, size: usize) -> u32 {
        Request::encode(dir::WRITE, n, size)
    }
    const fn io(n: u8) -> u32 {
        Request::encode(dir::NONE, n, 0)
    }
    pub const VERSION: u32 = iowr(nr::VERSION, super::Version::SIZE);
    pub const GET_CAP: u32 = iowr(nr::GET_CAP, super::GetCap::SIZE);
    pub const SET_CLIENT_CAP: u32 = iow(nr::SET_CLIENT_CAP, super::GetCap::SIZE);
    pub const SET_MASTER: u32 = io(nr::SET_MASTER);
    pub const DROP_MASTER: u32 = io(nr::DROP_MASTER);
    pub const GEM_CLOSE: u32 = iow(nr::GEM_CLOSE, super::GemClose::SIZE);
    pub const PRIME_HANDLE_TO_FD: u32 = iowr(nr::PRIME_HANDLE_TO_FD, super::PrimeHandle::SIZE);
    pub const PRIME_FD_TO_HANDLE: u32 = iowr(nr::PRIME_FD_TO_HANDLE, super::PrimeHandle::SIZE);
    pub const WAIT_VBLANK: u32 = iowr(nr::WAIT_VBLANK, super::WaitVblank::SIZE);
    pub const MODE_GETRESOURCES: u32 = iowr(nr::MODE_GETRESOURCES, super::CardRes::SIZE);
    pub const MODE_GETCRTC: u32 = iowr(nr::MODE_GETCRTC, super::ModeCrtc::SIZE);
    pub const MODE_SETCRTC: u32 = iowr(nr::MODE_SETCRTC, super::ModeCrtc::SIZE);
    pub const MODE_GETENCODER: u32 = iowr(nr::MODE_GETENCODER, super::GetEncoder::SIZE);
    pub const MODE_GETCONNECTOR: u32 = iowr(nr::MODE_GETCONNECTOR, super::GetConnector::SIZE);
    pub const MODE_GETPROPERTY: u32 = iowr(nr::MODE_GETPROPERTY, super::GetProperty::SIZE);
    pub const MODE_GETPROPBLOB: u32 = iowr(nr::MODE_GETPROPBLOB, super::GetBlob::SIZE);
    pub const MODE_ADDFB: u32 = iowr(nr::MODE_ADDFB, super::FbCmd::SIZE);
    pub const MODE_RMFB: u32 = iowr(nr::MODE_RMFB, 4);
    pub const MODE_PAGE_FLIP: u32 = iowr(nr::MODE_PAGE_FLIP, super::PageFlip::SIZE);
    pub const MODE_DIRTYFB: u32 = iowr(nr::MODE_DIRTYFB, super::FbDirty::SIZE);
    pub const MODE_CREATE_DUMB: u32 = iowr(nr::MODE_CREATE_DUMB, super::CreateDumb::SIZE);
    pub const MODE_MAP_DUMB: u32 = iowr(nr::MODE_MAP_DUMB, super::MapDumb::SIZE);
    pub const MODE_DESTROY_DUMB: u32 = iowr(nr::MODE_DESTROY_DUMB, 4);
    pub const MODE_GETPLANERESOURCES: u32 = iowr(nr::MODE_GETPLANERESOURCES, super::GetPlaneRes::SIZE);
    pub const MODE_GETPLANE: u32 = iowr(nr::MODE_GETPLANE, super::GetPlane::SIZE);
    pub const MODE_SETPLANE: u32 = iowr(nr::MODE_SETPLANE, super::SetPlane::SIZE);
    pub const MODE_ADDFB2: u32 = iowr(nr::MODE_ADDFB2, super::FbCmd2::SIZE);
    pub const MODE_GETFB2: u32 = iowr(nr::MODE_GETFB2, super::FbCmd2::SIZE);
    pub const MODE_OBJ_GETPROPERTIES: u32 = iowr(nr::MODE_OBJ_GETPROPERTIES, super::ObjGetProperties::SIZE);
    pub const MODE_OBJ_SETPROPERTY: u32 = iowr(nr::MODE_OBJ_SETPROPERTY, super::ObjSetProperty::SIZE);
    pub const MODE_ATOMIC: u32 = iowr(nr::MODE_ATOMIC, super::Atomic::SIZE);
    pub const MODE_CREATEPROPBLOB: u32 = iowr(nr::MODE_CREATEPROPBLOB, super::CreateBlob::SIZE);
    pub const MODE_DESTROYPROPBLOB: u32 = iowr(nr::MODE_DESTROYPROPBLOB, 4);
}

/// `DRM_CAP_*` (drm.h), for `DRM_IOCTL_GET_CAP`.
pub mod cap {
    pub const DUMB_BUFFER: u64 = 0x1;
    pub const VBLANK_HIGH_CRTC: u64 = 0x2;
    pub const DUMB_PREFERRED_DEPTH: u64 = 0x3;
    pub const DUMB_PREFER_SHADOW: u64 = 0x4;
    pub const PRIME: u64 = 0x5;
    pub const TIMESTAMP_MONOTONIC: u64 = 0x6;
    pub const ASYNC_PAGE_FLIP: u64 = 0x7;
    pub const CURSOR_WIDTH: u64 = 0x8;
    pub const CURSOR_HEIGHT: u64 = 0x9;
    pub const ADDFB2_MODIFIERS: u64 = 0x10;
    pub const PAGE_FLIP_TARGET: u64 = 0x11;
    pub const CRTC_IN_VBLANK_EVENT: u64 = 0x12;
    pub const SYNCOBJ: u64 = 0x13;
    pub const SYNCOBJ_TIMELINE: u64 = 0x14;
    pub const ATOMIC_ASYNC_PAGE_FLIP: u64 = 0x15;
    /// `DRM_PRIME_CAP_IMPORT`, `DRM_PRIME_CAP_EXPORT`.
    pub const PRIME_IMPORT: u64 = 0x1;
    pub const PRIME_EXPORT: u64 = 0x2;
}

/// `DRM_CLIENT_CAP_*` (drm.h), for `DRM_IOCTL_SET_CLIENT_CAP`.
pub mod client_cap {
    pub const STEREO_3D: u64 = 1;
    pub const UNIVERSAL_PLANES: u64 = 2;
    pub const ATOMIC: u64 = 3;
    pub const ASPECT_RATIO: u64 = 4;
    pub const WRITEBACK_CONNECTORS: u64 = 5;
    pub const CURSOR_PLANE_HOTSPOT: u64 = 6;
}

/// Event types and the vblank request bits (drm.h).
pub mod event {
    /// `DRM_EVENT_VBLANK`
    pub const VBLANK: u32 = 0x01;
    /// `DRM_EVENT_FLIP_COMPLETE`
    pub const FLIP_COMPLETE: u32 = 0x02;
    /// `DRM_EVENT_CRTC_SEQUENCE`
    pub const CRTC_SEQUENCE: u32 = 0x03;
    /// `struct drm_event_vblank` and `struct drm_event_crtc_sequence`: 32
    /// bytes each.
    pub const LEN: usize = 32;
}

/// `enum drm_vblank_seq_type` (drm.h).
pub mod vblank {
    pub const ABSOLUTE: u32 = 0x0;
    pub const RELATIVE: u32 = 0x1;
    pub const HIGH_CRTC_MASK: u32 = 0x0000_003e;
    pub const HIGH_CRTC_SHIFT: u32 = 1;
    pub const EVENT: u32 = 0x0400_0000;
    pub const FLIP: u32 = 0x0800_0000;
    pub const NEXTONMISS: u32 = 0x1000_0000;
    pub const SECONDARY: u32 = 0x2000_0000;
    pub const SIGNAL: u32 = 0x4000_0000;
    pub const TYPES_MASK: u32 = ABSOLUTE | RELATIVE;
    pub const FLAGS_MASK: u32 = EVENT | SIGNAL | SECONDARY | NEXTONMISS;
    /// `DRM_CRTC_SEQUENCE_RELATIVE`, `DRM_CRTC_SEQUENCE_NEXT_ON_MISS`.
    pub const SEQUENCE_RELATIVE: u32 = 0x1;
    pub const SEQUENCE_NEXT_ON_MISS: u32 = 0x2;
}

/// Flags of `DRM_IOCTL_PRIME_HANDLE_TO_FD` (drm.h: `DRM_CLOEXEC` is
/// `O_CLOEXEC`, `DRM_RDWR` is `O_RDWR`).
pub mod prime {
    pub const CLOEXEC: u32 = 0o2_000_000;
    pub const RDWR: u32 = 0o2;
}

/// Constants of drm_mode.h.
pub mod mode {
    pub const DISPLAY_MODE_LEN: usize = 32;
    pub const PROP_NAME_LEN: usize = 32;

    /// `DRM_MODE_TYPE_*`
    pub const TYPE_PREFERRED: u32 = 1 << 3;
    pub const TYPE_USERDEF: u32 = 1 << 5;
    pub const TYPE_DRIVER: u32 = 1 << 6;

    /// `DRM_MODE_FLAG_*`
    pub const FLAG_PHSYNC: u32 = 1 << 0;
    pub const FLAG_NHSYNC: u32 = 1 << 1;
    pub const FLAG_PVSYNC: u32 = 1 << 2;
    pub const FLAG_NVSYNC: u32 = 1 << 3;
    pub const FLAG_INTERLACE: u32 = 1 << 4;
    pub const FLAG_DBLSCAN: u32 = 1 << 5;

    /// `DRM_MODE_ENCODER_VIRTUAL`
    pub const ENCODER_VIRTUAL: u32 = 5;
    /// `DRM_MODE_CONNECTOR_VIRTUAL`
    pub const CONNECTOR_VIRTUAL: u32 = 15;

    /// `connection` of `struct drm_mode_get_connector` (`enum
    /// drm_connector_status`: connected 1, disconnected 2, unknown 3).
    pub const CONNECTED: u32 = 1;
    pub const DISCONNECTED: u32 = 2;
    /// `DRM_MODE_SUBPIXEL_UNKNOWN`
    pub const SUBPIXEL_UNKNOWN: u32 = 1;

    /// `DRM_MODE_PROP_*`
    pub const PROP_RANGE: u32 = 1 << 1;
    pub const PROP_IMMUTABLE: u32 = 1 << 2;
    pub const PROP_ENUM: u32 = 1 << 3;
    pub const PROP_BLOB: u32 = 1 << 4;
    pub const PROP_BITMASK: u32 = 1 << 5;
    /// `DRM_MODE_PROP_TYPE(1)`, `DRM_MODE_PROP_TYPE(2)`
    pub const PROP_OBJECT: u32 = 1 << 6;
    pub const PROP_SIGNED_RANGE: u32 = 2 << 6;
    pub const PROP_ATOMIC: u32 = 0x8000_0000;

    /// `DRM_MODE_OBJECT_*`
    pub const OBJECT_CRTC: u32 = 0xcccc_cccc;
    pub const OBJECT_CONNECTOR: u32 = 0xc0c0_c0c0;
    pub const OBJECT_ENCODER: u32 = 0xe0e0_e0e0;
    pub const OBJECT_MODE: u32 = 0xdede_dede;
    pub const OBJECT_PROPERTY: u32 = 0xb0b0_b0b0;
    pub const OBJECT_FB: u32 = 0xfbfb_fbfb;
    pub const OBJECT_BLOB: u32 = 0xbbbb_bbbb;
    pub const OBJECT_PLANE: u32 = 0xeeee_eeee;
    pub const OBJECT_ANY: u32 = 0;

    /// `DRM_PLANE_TYPE_*` values of the plane `type` property.
    pub const PLANE_OVERLAY: u64 = 0;
    pub const PLANE_PRIMARY: u64 = 1;
    pub const PLANE_CURSOR: u64 = 2;

    /// `DRM_MODE_DPMS_*`
    pub const DPMS_ON: u64 = 0;
    pub const DPMS_STANDBY: u64 = 1;
    pub const DPMS_SUSPEND: u64 = 2;
    pub const DPMS_OFF: u64 = 3;

    /// `DRM_MODE_LINK_STATUS_*`
    pub const LINK_STATUS_GOOD: u64 = 0;
    pub const LINK_STATUS_BAD: u64 = 1;

    /// `DRM_MODE_PAGE_FLIP_*`
    pub const PAGE_FLIP_EVENT: u32 = 0x01;
    pub const PAGE_FLIP_ASYNC: u32 = 0x02;
    pub const PAGE_FLIP_TARGET_ABSOLUTE: u32 = 0x4;
    pub const PAGE_FLIP_TARGET_RELATIVE: u32 = 0x8;
    pub const PAGE_FLIP_FLAGS: u32 =
        PAGE_FLIP_EVENT | PAGE_FLIP_ASYNC | PAGE_FLIP_TARGET_ABSOLUTE | PAGE_FLIP_TARGET_RELATIVE;

    /// `DRM_MODE_ATOMIC_*`
    pub const ATOMIC_TEST_ONLY: u32 = 0x0100;
    pub const ATOMIC_NONBLOCK: u32 = 0x0200;
    pub const ATOMIC_ALLOW_MODESET: u32 = 0x0400;
    pub const ATOMIC_FLAGS: u32 =
        PAGE_FLIP_EVENT | PAGE_FLIP_ASYNC | ATOMIC_TEST_ONLY | ATOMIC_NONBLOCK | ATOMIC_ALLOW_MODESET;

    /// `DRM_MODE_FB_*`
    pub const FB_INTERLACED: u32 = 1 << 0;
    pub const FB_MODIFIERS: u32 = 1 << 1;

    /// `DRM_MODE_FB_DIRTY_*`
    pub const FB_DIRTY_ANNOTATE_COPY: u32 = 0x01;
    pub const FB_DIRTY_ANNOTATE_FILL: u32 = 0x02;
    pub const FB_DIRTY_FLAGS: u32 = 0x03;
    pub const FB_DIRTY_MAX_CLIPS: u32 = 256;

    /// `FORMAT_BLOB_CURRENT` (the version of `struct
    /// drm_format_modifier_blob`).
    pub const FORMAT_BLOB_CURRENT: u32 = 1;
}

/// Pixel formats and modifiers (drm_fourcc.h: `fourcc_code(a, b, c, d)` is
/// `a | b << 8 | c << 16 | d << 24`).
pub mod fourcc {
    pub const fn code(a: u8, b: u8, c: u8, d: u8) -> u32 {
        a as u32 | (b as u32) << 8 | (c as u32) << 16 | (d as u32) << 24
    }
    /// `[31:0] x:R:G:B 8:8:8:8 little endian`
    pub const XRGB8888: u32 = code(b'X', b'R', b'2', b'4');
    /// `[31:0] A:R:G:B 8:8:8:8 little endian`
    pub const ARGB8888: u32 = code(b'A', b'R', b'2', b'4');
    /// `[31:0] x:B:G:R 8:8:8:8 little endian`
    pub const XBGR8888: u32 = code(b'X', b'B', b'2', b'4');
    /// `[31:0] A:B:G:R 8:8:8:8 little endian`
    pub const ABGR8888: u32 = code(b'A', b'B', b'2', b'4');
    /// `[15:0] R:G:B 5:6:5 little endian`
    pub const RGB565: u32 = code(b'R', b'G', b'1', b'6');
    /// `DRM_FORMAT_MOD_LINEAR`
    pub const MOD_LINEAR: u64 = 0;
    /// `DRM_FORMAT_MOD_INVALID`
    pub const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
}

/// A field of a uapi structure at a byte offset.
pub trait Field: Sized {
    fn get(b: &[u8], at: usize) -> Self;
    fn put(&self, b: &mut [u8], at: usize);
}

macro_rules! int_field {
    ($($t:ty),*) => {$(
        impl Field for $t {
            fn get(b: &[u8], at: usize) -> $t {
                const N: usize = core::mem::size_of::<$t>();
                let mut a = [0u8; N];
                a.copy_from_slice(&b[at..at + N]);
                <$t>::from_le_bytes(a)
            }
            fn put(&self, b: &mut [u8], at: usize) {
                let a = self.to_le_bytes();
                b[at..at + a.len()].copy_from_slice(&a);
            }
        }
    )*};
}
int_field!(u16, u32, u64, i32, i64);

impl<T: Field + Copy + Default, const N: usize> Field for [T; N] {
    fn get(b: &[u8], at: usize) -> [T; N] {
        let mut out = [T::default(); N];
        let size = core::mem::size_of::<T>();
        for (i, v) in out.iter_mut().enumerate() {
            *v = T::get(b, at + i * size);
        }
        out
    }
    fn put(&self, b: &mut [u8], at: usize) {
        let size = core::mem::size_of::<T>();
        for (i, v) in self.iter().enumerate() {
            v.put(b, at + i * size);
        }
    }
}

impl Field for u8 {
    fn get(b: &[u8], at: usize) -> u8 {
        b[at]
    }
    fn put(&self, b: &mut [u8], at: usize) {
        b[at] = *self;
    }
}

/// A uapi structure: its fields at the header's offsets (`@`), its size,
/// and decoding from / encoding into argument bytes (at least `SIZE`).
macro_rules! layout {
    ($(#[$m:meta])* $name:ident [$size:expr] { $($(#[$fm:meta])* $field:ident : $ty:ty = $off:expr),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct $name { $($(#[$fm])* pub $field: $ty),* }
        impl $name {
            pub const SIZE: usize = $size;
            pub fn decode(b: &[u8]) -> $name {
                $name { $($field: <$ty as Field>::get(b, $off)),* }
            }
            pub fn encode(&self, b: &mut [u8]) {
                $(<$ty as Field>::put(&self.$field, b, $off);)*
            }
            pub fn to_bytes(&self) -> [u8; $size] {
                let mut b = [0u8; $size];
                self.encode(&mut b);
                b
            }
        }
    };
}

layout! {
    /// `struct drm_version`.
    Version [64] {
        major: i32 = 0, minor: i32 = 4, patchlevel: i32 = 8,
        name_len: u64 = 16, name: u64 = 24,
        date_len: u64 = 32, date: u64 = 40,
        desc_len: u64 = 48, desc: u64 = 56,
    }
}

layout! {
    /// `struct drm_unique`.
    Unique [16] { len: u64 = 0, ptr: u64 = 8 }
}

layout! {
    /// `struct drm_auth`.
    Auth [4] { magic: u32 = 0 }
}

layout! {
    /// `struct drm_set_version`.
    SetVersion [16] { di_major: i32 = 0, di_minor: i32 = 4, dd_major: i32 = 8, dd_minor: i32 = 12 }
}

layout! {
    /// `struct drm_gem_close`.
    GemClose [8] { handle: u32 = 0, pad: u32 = 4 }
}

layout! {
    /// `struct drm_gem_flink`.
    GemFlink [8] { handle: u32 = 0, name: u32 = 4 }
}

layout! {
    /// `struct drm_gem_open`.
    GemOpen [16] { name: u32 = 0, handle: u32 = 4, size: u64 = 8 }
}

layout! {
    /// `struct drm_get_cap` (and `struct drm_set_client_cap`, the same
    /// layout).
    GetCap [16] { capability: u64 = 0, value: u64 = 8 }
}

layout! {
    /// `struct drm_prime_handle`.
    PrimeHandle [12] { handle: u32 = 0, flags: u32 = 4, fd: i32 = 8 }
}

layout! {
    /// `union drm_wait_vblank`: the request (`type`, `sequence`, `signal`)
    /// and the reply (`type`, `sequence`, `tval_sec`, `tval_usec`) overlap.
    WaitVblank [24] { ty: u32 = 0, sequence: u32 = 4, signal: u64 = 8, tval_usec: i64 = 16 }
}

layout! {
    /// `struct drm_crtc_get_sequence`.
    CrtcGetSequence [24] { crtc_id: u32 = 0, active: u32 = 4, sequence: u64 = 8, sequence_ns: i64 = 16 }
}

layout! {
    /// `struct drm_crtc_queue_sequence`.
    CrtcQueueSequence [24] { crtc_id: u32 = 0, flags: u32 = 4, sequence: u64 = 8, user_data: u64 = 16 }
}

layout! {
    /// `struct drm_mode_modeinfo`.
    ModeInfo [68] {
        clock: u32 = 0,
        hdisplay: u16 = 4, hsync_start: u16 = 6, hsync_end: u16 = 8, htotal: u16 = 10, hskew: u16 = 12,
        vdisplay: u16 = 14, vsync_start: u16 = 16, vsync_end: u16 = 18, vtotal: u16 = 20, vscan: u16 = 22,
        vrefresh: u32 = 24, flags: u32 = 28, ty: u32 = 32,
        name: [u8; 32] = 36,
    }
}

layout! {
    /// `struct drm_mode_card_res`.
    CardRes [64] {
        fb_id_ptr: u64 = 0, crtc_id_ptr: u64 = 8, connector_id_ptr: u64 = 16, encoder_id_ptr: u64 = 24,
        count_fbs: u32 = 32, count_crtcs: u32 = 36, count_connectors: u32 = 40, count_encoders: u32 = 44,
        min_width: u32 = 48, max_width: u32 = 52, min_height: u32 = 56, max_height: u32 = 60,
    }
}

layout! {
    /// `struct drm_mode_crtc`.
    ModeCrtc [104] {
        set_connectors_ptr: u64 = 0, count_connectors: u32 = 8,
        crtc_id: u32 = 12, fb_id: u32 = 16, x: u32 = 20, y: u32 = 24,
        gamma_size: u32 = 28, mode_valid: u32 = 32,
        mode: ModeInfo = 36,
    }
}

impl Field for ModeInfo {
    fn get(b: &[u8], at: usize) -> ModeInfo {
        ModeInfo::decode(&b[at..])
    }
    fn put(&self, b: &mut [u8], at: usize) {
        self.encode(&mut b[at..]);
    }
}

layout! {
    /// `struct drm_mode_cursor` (the start of `struct drm_mode_cursor2`).
    Cursor [28] { flags: u32 = 0, crtc_id: u32 = 4, x: i32 = 8, y: i32 = 12, width: u32 = 16, height: u32 = 20, handle: u32 = 24 }
}

layout! {
    /// `struct drm_mode_crtc_lut`.
    CrtcLut [32] { crtc_id: u32 = 0, gamma_size: u32 = 4, red: u64 = 8, green: u64 = 16, blue: u64 = 24 }
}

layout! {
    /// `struct drm_mode_get_encoder`.
    GetEncoder [20] { encoder_id: u32 = 0, encoder_type: u32 = 4, crtc_id: u32 = 8, possible_crtcs: u32 = 12, possible_clones: u32 = 16 }
}

layout! {
    /// `struct drm_mode_get_connector`.
    GetConnector [80] {
        encoders_ptr: u64 = 0, modes_ptr: u64 = 8, props_ptr: u64 = 16, prop_values_ptr: u64 = 24,
        count_modes: u32 = 32, count_props: u32 = 36, count_encoders: u32 = 40,
        encoder_id: u32 = 44, connector_id: u32 = 48, connector_type: u32 = 52, connector_type_id: u32 = 56,
        connection: u32 = 60, mm_width: u32 = 64, mm_height: u32 = 68, subpixel: u32 = 72, pad: u32 = 76,
    }
}

layout! {
    /// `struct drm_mode_get_property`.
    GetProperty [64] {
        values_ptr: u64 = 0, enum_blob_ptr: u64 = 8, prop_id: u32 = 16, flags: u32 = 20,
        name: [u8; 32] = 24, count_values: u32 = 56, count_enum_blobs: u32 = 60,
    }
}

layout! {
    /// `struct drm_mode_property_enum`.
    PropertyEnum [40] { value: u64 = 0, name: [u8; 32] = 8 }
}

layout! {
    /// `struct drm_mode_connector_set_property`.
    ConnectorSetProperty [16] { value: u64 = 0, prop_id: u32 = 8, connector_id: u32 = 12 }
}

layout! {
    /// `struct drm_mode_obj_get_properties`.
    ObjGetProperties [32] { props_ptr: u64 = 0, prop_values_ptr: u64 = 8, count_props: u32 = 16, obj_id: u32 = 20, obj_type: u32 = 24 }
}

layout! {
    /// `struct drm_mode_obj_set_property`.
    ObjSetProperty [24] { value: u64 = 0, prop_id: u32 = 8, obj_id: u32 = 12, obj_type: u32 = 16 }
}

layout! {
    /// `struct drm_mode_get_blob`.
    GetBlob [16] { blob_id: u32 = 0, length: u32 = 4, data: u64 = 8 }
}

layout! {
    /// `struct drm_mode_fb_cmd`.
    FbCmd [28] { fb_id: u32 = 0, width: u32 = 4, height: u32 = 8, pitch: u32 = 12, bpp: u32 = 16, depth: u32 = 20, handle: u32 = 24 }
}

layout! {
    /// `struct drm_mode_fb_cmd2`.
    FbCmd2 [104] {
        fb_id: u32 = 0, width: u32 = 4, height: u32 = 8, pixel_format: u32 = 12, flags: u32 = 16,
        handles: [u32; 4] = 20, pitches: [u32; 4] = 36, offsets: [u32; 4] = 52, modifier: [u64; 4] = 72,
    }
}

layout! {
    /// `struct drm_mode_fb_dirty_cmd`.
    FbDirty [24] { fb_id: u32 = 0, flags: u32 = 4, color: u32 = 8, num_clips: u32 = 12, clips_ptr: u64 = 16 }
}

layout! {
    /// `struct drm_clip_rect` (drm.h).
    ClipRect [8] { x1: u16 = 0, y1: u16 = 2, x2: u16 = 4, y2: u16 = 6 }
}

layout! {
    /// `struct drm_mode_crtc_page_flip_target` (`struct
    /// drm_mode_crtc_page_flip` with `reserved` as `sequence`).
    PageFlip [24] { crtc_id: u32 = 0, fb_id: u32 = 4, flags: u32 = 8, sequence: u32 = 12, user_data: u64 = 16 }
}

layout! {
    /// `struct drm_mode_create_dumb`.
    CreateDumb [32] { height: u32 = 0, width: u32 = 4, bpp: u32 = 8, flags: u32 = 12, handle: u32 = 16, pitch: u32 = 20, size: u64 = 24 }
}

layout! {
    /// `struct drm_mode_map_dumb`.
    MapDumb [16] { handle: u32 = 0, pad: u32 = 4, offset: u64 = 8 }
}

layout! {
    /// `struct drm_mode_get_plane_res`.
    GetPlaneRes [16] { plane_id_ptr: u64 = 0, count_planes: u32 = 8 }
}

layout! {
    /// `struct drm_mode_get_plane`.
    GetPlane [32] {
        plane_id: u32 = 0, crtc_id: u32 = 4, fb_id: u32 = 8, possible_crtcs: u32 = 12,
        gamma_size: u32 = 16, count_format_types: u32 = 20, format_type_ptr: u64 = 24,
    }
}

layout! {
    /// `struct drm_mode_set_plane` (source values 16.16 fixed point; note
    /// the header's order `src_h, src_w`).
    SetPlane [48] {
        plane_id: u32 = 0, crtc_id: u32 = 4, fb_id: u32 = 8, flags: u32 = 12,
        crtc_x: i32 = 16, crtc_y: i32 = 20, crtc_w: u32 = 24, crtc_h: u32 = 28,
        src_x: u32 = 32, src_y: u32 = 36, src_h: u32 = 40, src_w: u32 = 44,
    }
}

layout! {
    /// `struct drm_mode_atomic`.
    Atomic [56] {
        flags: u32 = 0, count_objs: u32 = 4, objs_ptr: u64 = 8, count_props_ptr: u64 = 16,
        props_ptr: u64 = 24, prop_values_ptr: u64 = 32, reserved: u64 = 40, user_data: u64 = 48,
    }
}

layout! {
    /// `struct drm_mode_create_blob`.
    CreateBlob [16] { data: u64 = 0, length: u32 = 8, blob_id: u32 = 12 }
}

layout! {
    /// `struct drm_event_vblank`.
    EventVblank [32] { ty: u32 = 0, length: u32 = 4, user_data: u64 = 8, tv_sec: u32 = 16, tv_usec: u32 = 20, sequence: u32 = 24, crtc_id: u32 = 28 }
}

layout! {
    /// `struct drm_event_crtc_sequence`.
    EventCrtcSequence [32] { ty: u32 = 0, length: u32 = 4, user_data: u64 = 8, time_ns: i64 = 16, sequence: u64 = 24 }
}

layout! {
    /// `struct drm_format_modifier_blob` (the header of an `IN_FORMATS`
    /// blob).
    FormatModifierBlob [24] {
        version: u32 = 0, flags: u32 = 4, count_formats: u32 = 8, formats_offset: u32 = 12,
        count_modifiers: u32 = 16, modifiers_offset: u32 = 20,
    }
}

layout! {
    /// `struct drm_format_modifier`.
    FormatModifier [24] { formats: u64 = 0, offset: u32 = 8, pad: u32 = 12, modifier: u64 = 16 }
}

/// The NUL-terminated string in a fixed-size name field.
pub fn name_bytes<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    let n = s.len().min(N - 1);
    out[..n].copy_from_slice(&s.as_bytes()[..n]);
    out
}

/// The string in a NUL-terminated name field.
pub fn name_str(b: &[u8]) -> &str {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..n]).unwrap_or("")
}

/// The virtio-gpu driver's own ioctls and their structures, from the
/// MIT-licensed uapi header `drm/virtgpu_drm.h`: request numbers
/// `DRM_COMMAND_BASE` (0x40) + `DRM_VIRTGPU_*`, every one `DRM_IOWR`.
/// Mesa's virgl (OpenGL) and Venus (Vulkan) drivers speak this interface
/// (ADR 0119).
pub mod virtgpu {
    use super::{Field, Request, dir};

    /// `DRM_COMMAND_BASE`: driver-specific requests start here.
    pub const COMMAND_BASE: u8 = 0x40;
    /// `DRM_COMMAND_END`: and end before here (the mode-setting requests).
    pub const COMMAND_END: u8 = 0xa0;

    /// `DRM_VIRTGPU_*` + [`COMMAND_BASE`].
    pub mod nr {
        use super::COMMAND_BASE;
        pub const MAP: u8 = COMMAND_BASE + 0x01;
        pub const EXECBUFFER: u8 = COMMAND_BASE + 0x02;
        pub const GETPARAM: u8 = COMMAND_BASE + 0x03;
        pub const RESOURCE_CREATE: u8 = COMMAND_BASE + 0x04;
        pub const RESOURCE_INFO: u8 = COMMAND_BASE + 0x05;
        pub const TRANSFER_FROM_HOST: u8 = COMMAND_BASE + 0x06;
        pub const TRANSFER_TO_HOST: u8 = COMMAND_BASE + 0x07;
        pub const WAIT: u8 = COMMAND_BASE + 0x08;
        pub const GET_CAPS: u8 = COMMAND_BASE + 0x09;
        pub const RESOURCE_CREATE_BLOB: u8 = COMMAND_BASE + 0x0a;
        pub const CONTEXT_INIT: u8 = COMMAND_BASE + 0x0b;
    }

    /// `VIRTGPU_EXECBUF_*`.
    pub mod execbuf {
        pub const FENCE_FD_IN: u32 = 0x01;
        pub const FENCE_FD_OUT: u32 = 0x02;
        pub const RING_IDX: u32 = 0x04;
        pub const FLAGS: u32 = FENCE_FD_IN | FENCE_FD_OUT | RING_IDX;
    }

    /// `VIRTGPU_PARAM_*` of `DRM_IOCTL_VIRTGPU_GETPARAM`.
    pub mod param {
        /// 3D features in the device (virgl).
        pub const THREE_D_FEATURES: u64 = 1;
        /// `GET_CAPS` takes the capset ID and version as they are.
        pub const CAPSET_QUERY_FIX: u64 = 2;
        pub const RESOURCE_BLOB: u64 = 3;
        /// Host blob resources can be mapped.
        pub const HOST_VISIBLE: u64 = 4;
        pub const CROSS_DEVICE: u64 = 5;
        pub const CONTEXT_INIT: u64 = 6;
        /// A bit per capset ID the device supports.
        pub const SUPPORTED_CAPSET_IDS: u64 = 7;
        pub const EXPLICIT_DEBUG_NAME: u64 = 8;
    }

    /// `VIRTGPU_WAIT_NOWAIT`.
    pub const WAIT_NOWAIT: u32 = 1;

    /// `VIRTGPU_BLOB_MEM_*` and `VIRTGPU_BLOB_FLAG_*`.
    pub mod blob {
        pub const MEM_GUEST: u32 = 0x0001;
        pub const MEM_HOST3D: u32 = 0x0002;
        pub const MEM_HOST3D_GUEST: u32 = 0x0003;
        pub const FLAG_USE_MAPPABLE: u32 = 0x0001;
        pub const FLAG_USE_SHAREABLE: u32 = 0x0002;
        pub const FLAG_USE_CROSS_DEVICE: u32 = 0x0004;
        pub const FLAGS: u32 = FLAG_USE_MAPPABLE | FLAG_USE_SHAREABLE | FLAG_USE_CROSS_DEVICE;
    }

    /// `VIRTGPU_CONTEXT_PARAM_*` of `DRM_IOCTL_VIRTGPU_CONTEXT_INIT`.
    pub mod context_param {
        pub const CAPSET_ID: u64 = 0x0001;
        pub const NUM_RINGS: u64 = 0x0002;
        pub const POLL_RINGS_MASK: u64 = 0x0003;
        pub const DEBUG_NAME: u64 = 0x0004;
    }

    layout! {
        /// `struct drm_virtgpu_map`.
        Map [16] { offset: u64 = 0, handle: u32 = 8, pad: u32 = 12 }
    }

    layout! {
        /// `struct drm_virtgpu_execbuffer` (40 bytes before the syncobj
        /// fields were added).
        Execbuffer [64] {
            flags: u32 = 0, size: u32 = 4, command: u64 = 8, bo_handles: u64 = 16,
            num_bo_handles: u32 = 24, fence_fd: i32 = 28, ring_idx: u32 = 32,
            syncobj_stride: u32 = 36, num_in_syncobjs: u32 = 40, num_out_syncobjs: u32 = 44,
            in_syncobjs: u64 = 48, out_syncobjs: u64 = 56,
        }
    }

    layout! {
        /// `struct drm_virtgpu_getparam`: `value` points at an `int`.
        GetParam [16] { param: u64 = 0, value: u64 = 8 }
    }

    layout! {
        /// `struct drm_virtgpu_resource_create`.
        ResourceCreate [56] {
            target: u32 = 0, format: u32 = 4, bind: u32 = 8, width: u32 = 12, height: u32 = 16,
            depth: u32 = 20, array_size: u32 = 24, last_level: u32 = 28, nr_samples: u32 = 32,
            flags: u32 = 36, bo_handle: u32 = 40, res_handle: u32 = 44, size: u32 = 48, stride: u32 = 52,
        }
    }

    layout! {
        /// `struct drm_virtgpu_resource_info`.
        ResourceInfo [16] { bo_handle: u32 = 0, res_handle: u32 = 4, size: u32 = 8, blob_mem: u32 = 12 }
    }

    layout! {
        /// `struct drm_virtgpu_3d_transfer_to_host` and `..._from_host`
        /// (the same layout; 36 bytes before `stride` and `layer_stride`),
        /// with its `struct drm_virtgpu_3d_box` spelled out.
        Transfer [44] {
            bo_handle: u32 = 0, x: u32 = 4, y: u32 = 8, z: u32 = 12, w: u32 = 16, h: u32 = 20, d: u32 = 24,
            level: u32 = 28, offset: u32 = 32, stride: u32 = 36, layer_stride: u32 = 40,
        }
    }

    layout! {
        /// `struct drm_virtgpu_3d_wait`.
        Wait [8] { handle: u32 = 0, flags: u32 = 4 }
    }

    layout! {
        /// `struct drm_virtgpu_get_caps`.
        GetCaps [24] { cap_set_id: u32 = 0, cap_set_ver: u32 = 4, addr: u64 = 8, size: u32 = 16, pad: u32 = 20 }
    }

    layout! {
        /// `struct drm_virtgpu_resource_create_blob`.
        ResourceCreateBlob [48] {
            blob_mem: u32 = 0, blob_flags: u32 = 4, bo_handle: u32 = 8, res_handle: u32 = 12, size: u64 = 16,
            pad: u32 = 24, cmd_size: u32 = 28, cmd: u64 = 32, blob_id: u64 = 40,
        }
    }

    layout! {
        /// `struct drm_virtgpu_context_init`.
        ContextInit [16] { num_params: u32 = 0, pad: u32 = 4, ctx_set_params: u64 = 8 }
    }

    layout! {
        /// `struct drm_virtgpu_context_set_param`.
        ContextSetParam [16] { param: u64 = 0, value: u64 = 8 }
    }

    /// `DRM_IOCTL_VIRTGPU_*` request numbers as userspace passes them.
    pub mod ioctl {
        use super::*;
        const fn iowr(n: u8, size: usize) -> u32 {
            Request::encode(dir::READ | dir::WRITE, n, size)
        }
        pub const MAP: u32 = iowr(nr::MAP, Map::SIZE);
        pub const EXECBUFFER: u32 = iowr(nr::EXECBUFFER, Execbuffer::SIZE);
        pub const GETPARAM: u32 = iowr(nr::GETPARAM, GetParam::SIZE);
        pub const RESOURCE_CREATE: u32 = iowr(nr::RESOURCE_CREATE, ResourceCreate::SIZE);
        pub const RESOURCE_INFO: u32 = iowr(nr::RESOURCE_INFO, ResourceInfo::SIZE);
        pub const TRANSFER_FROM_HOST: u32 = iowr(nr::TRANSFER_FROM_HOST, Transfer::SIZE);
        pub const TRANSFER_TO_HOST: u32 = iowr(nr::TRANSFER_TO_HOST, Transfer::SIZE);
        pub const WAIT: u32 = iowr(nr::WAIT, Wait::SIZE);
        pub const GET_CAPS: u32 = iowr(nr::GET_CAPS, GetCaps::SIZE);
        pub const RESOURCE_CREATE_BLOB: u32 = iowr(nr::RESOURCE_CREATE_BLOB, ResourceCreateBlob::SIZE);
        pub const CONTEXT_INIT: u32 = iowr(nr::CONTEXT_INIT, ContextInit::SIZE);
    }
}
