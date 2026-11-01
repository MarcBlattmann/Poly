//! KMS properties: the standard ones a 2D device has, with the names,
//! types and ranges of the kernel's "KMS Properties" documentation.
//!
//! | Object | Property | Type |
//! |---|---|---|
//! | connector | `EDID` | blob, immutable |
//! | connector | `DPMS` | enum On / Standby / Suspend / Off |
//! | connector | `link-status` | enum Good / Bad |
//! | connector | `non-desktop` | range 0-1, immutable |
//! | connector, plane | `CRTC_ID` | object (CRTC), atomic |
//! | CRTC | `ACTIVE` | range 0-1, atomic |
//! | CRTC | `MODE_ID` | blob (`struct drm_mode_modeinfo`), atomic |
//! | plane | `type` | enum Overlay / Primary / Cursor, immutable |
//! | plane | `FB_ID` | object (framebuffer), atomic |
//! | plane | `SRC_X`, `SRC_Y`, `SRC_W`, `SRC_H` | range, 16.16 fixed point, atomic |
//! | plane | `CRTC_X`, `CRTC_Y` | signed range, atomic |
//! | plane | `CRTC_W`, `CRTC_H` | range, atomic |
//! | plane | `IN_FORMATS` | blob (`struct drm_format_modifier_blob`), immutable |
//!
//! Atomic properties (`DRM_MODE_PROP_ATOMIC`) are listed only to clients
//! that set `DRM_CLIENT_CAP_ATOMIC`.

use crate::uapi::mode as m;

/// A property the device has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prop {
    Edid,
    Dpms,
    LinkStatus,
    NonDesktop,
    CrtcId,
    Active,
    ModeId,
    Type,
    FbId,
    SrcX,
    SrcY,
    SrcW,
    SrcH,
    CrtcX,
    CrtcY,
    CrtcW,
    CrtcH,
    InFormats,
}

/// What kind of values a property takes.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Range(u64, u64),
    SignedRange(i64, i64),
    Enum(&'static [(u64, &'static str)]),
    Blob,
    Object(u32),
}

impl Prop {
    pub const ALL: [Prop; 18] = [
        Prop::Edid,
        Prop::Dpms,
        Prop::LinkStatus,
        Prop::NonDesktop,
        Prop::CrtcId,
        Prop::Active,
        Prop::ModeId,
        Prop::Type,
        Prop::FbId,
        Prop::SrcX,
        Prop::SrcY,
        Prop::SrcW,
        Prop::SrcH,
        Prop::CrtcX,
        Prop::CrtcY,
        Prop::CrtcW,
        Prop::CrtcH,
        Prop::InFormats,
    ];

    /// The properties of a connector, a CRTC and a plane, in the order they
    /// are listed.
    pub const CONNECTOR: [Prop; 5] = [Prop::Edid, Prop::Dpms, Prop::LinkStatus, Prop::NonDesktop, Prop::CrtcId];
    pub const CRTC: [Prop; 2] = [Prop::Active, Prop::ModeId];
    pub const PLANE: [Prop; 12] = [
        Prop::Type,
        Prop::FbId,
        Prop::CrtcId,
        Prop::SrcX,
        Prop::SrcY,
        Prop::SrcW,
        Prop::SrcH,
        Prop::CrtcX,
        Prop::CrtcY,
        Prop::CrtcW,
        Prop::CrtcH,
        Prop::InFormats,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn name(self) -> &'static str {
        match self {
            Prop::Edid => "EDID",
            Prop::Dpms => "DPMS",
            Prop::LinkStatus => "link-status",
            Prop::NonDesktop => "non-desktop",
            Prop::CrtcId => "CRTC_ID",
            Prop::Active => "ACTIVE",
            Prop::ModeId => "MODE_ID",
            Prop::Type => "type",
            Prop::FbId => "FB_ID",
            Prop::SrcX => "SRC_X",
            Prop::SrcY => "SRC_Y",
            Prop::SrcW => "SRC_W",
            Prop::SrcH => "SRC_H",
            Prop::CrtcX => "CRTC_X",
            Prop::CrtcY => "CRTC_Y",
            Prop::CrtcW => "CRTC_W",
            Prop::CrtcH => "CRTC_H",
            Prop::InFormats => "IN_FORMATS",
        }
    }

    pub const fn kind(self) -> Kind {
        const DPMS: [(u64, &str); 4] =
            [(m::DPMS_ON, "On"), (m::DPMS_STANDBY, "Standby"), (m::DPMS_SUSPEND, "Suspend"), (m::DPMS_OFF, "Off")];
        const LINK: [(u64, &str); 2] = [(m::LINK_STATUS_GOOD, "Good"), (m::LINK_STATUS_BAD, "Bad")];
        const TYPE: [(u64, &str); 3] =
            [(m::PLANE_OVERLAY, "Overlay"), (m::PLANE_PRIMARY, "Primary"), (m::PLANE_CURSOR, "Cursor")];
        match self {
            Prop::Edid | Prop::ModeId | Prop::InFormats => Kind::Blob,
            Prop::Dpms => Kind::Enum(&DPMS),
            Prop::LinkStatus => Kind::Enum(&LINK),
            Prop::Type => Kind::Enum(&TYPE),
            Prop::NonDesktop | Prop::Active => Kind::Range(0, 1),
            Prop::CrtcId => Kind::Object(m::OBJECT_CRTC),
            Prop::FbId => Kind::Object(m::OBJECT_FB),
            Prop::SrcX | Prop::SrcY | Prop::SrcW | Prop::SrcH => Kind::Range(0, u32::MAX as u64),
            Prop::CrtcX | Prop::CrtcY => Kind::SignedRange(i32::MIN as i64, i32::MAX as i64),
            Prop::CrtcW | Prop::CrtcH => Kind::Range(0, i32::MAX as u64),
        }
    }

    pub const fn immutable(self) -> bool {
        matches!(self, Prop::Edid | Prop::NonDesktop | Prop::Type | Prop::InFormats)
    }

    pub const fn atomic(self) -> bool {
        matches!(
            self,
            Prop::CrtcId
                | Prop::Active
                | Prop::ModeId
                | Prop::FbId
                | Prop::SrcX
                | Prop::SrcY
                | Prop::SrcW
                | Prop::SrcH
                | Prop::CrtcX
                | Prop::CrtcY
                | Prop::CrtcW
                | Prop::CrtcH
        )
    }

    /// The `flags` `DRM_IOCTL_MODE_GETPROPERTY` reports.
    pub const fn flags(self) -> u32 {
        let ty = match self.kind() {
            Kind::Range(..) => m::PROP_RANGE,
            Kind::SignedRange(..) => m::PROP_SIGNED_RANGE,
            Kind::Enum(_) => m::PROP_ENUM,
            Kind::Blob => m::PROP_BLOB,
            Kind::Object(_) => m::PROP_OBJECT,
        };
        ty | if self.immutable() { m::PROP_IMMUTABLE } else { 0 } | if self.atomic() { m::PROP_ATOMIC } else { 0 }
    }

    /// Whether `value` is one the property takes (objects and blobs are
    /// checked by the caller, which knows what exists).
    pub fn accepts(self, value: u64) -> bool {
        match self.kind() {
            Kind::Range(lo, hi) => (lo..=hi).contains(&value),
            Kind::SignedRange(lo, hi) => (lo..=hi).contains(&(value as i64)),
            Kind::Enum(values) => values.iter().any(|(v, _)| *v == value),
            Kind::Blob | Kind::Object(_) => value <= u32::MAX as u64,
        }
    }
}
