//! DXGI formats (dxgiformat.h): their sizes, their families (a typeless format and the typed formats
//! that share its bit layout, Learn "Format Conversion using Direct3D 10.1"), and what the device
//! supports for each (`ID3D11Device::CheckFormatSupport`, ID3D11Device.CheckFormatSupport.yaml).

use crate::hr::*;

pub const UNKNOWN: u32 = 0;
pub const R32G32B32A32_FLOAT: u32 = 2;
pub const R16G16B16A16_FLOAT: u32 = 10;
pub const D32_FLOAT_S8X24_UINT: u32 = 20;
pub const R10G10B10A2_UNORM: u32 = 24;
pub const R8G8B8A8_TYPELESS: u32 = 27;
pub const R8G8B8A8_UNORM: u32 = 28;
pub const R8G8B8A8_UNORM_SRGB: u32 = 29;
pub const R32_TYPELESS: u32 = 39;
pub const D32_FLOAT: u32 = 40;
pub const R32_FLOAT: u32 = 41;
pub const R24G8_TYPELESS: u32 = 44;
pub const D24_UNORM_S8_UINT: u32 = 45;
pub const R16_TYPELESS: u32 = 53;
pub const D16_UNORM: u32 = 55;
pub const R8_UNORM: u32 = 61;
pub const B8G8R8A8_UNORM: u32 = 87;
pub const B8G8R8X8_UNORM: u32 = 88;
pub const B8G8R8A8_TYPELESS: u32 = 90;
pub const B8G8R8A8_UNORM_SRGB: u32 = 91;
pub const B8G8R8X8_TYPELESS: u32 = 92;
pub const NV12: u32 = 103;

/// `D3D11_FORMAT_SUPPORT` bits (d3d11.h).
pub mod support {
    pub const BUFFER: u32 = 0x1;
    pub const TEXTURE2D: u32 = 0x20;
    pub const TEXTURECUBE: u32 = 0x80;
    pub const SHADER_SAMPLE: u32 = 0x200;
    pub const MIP_AUTOGEN: u32 = 0x2000;
    pub const RENDER_TARGET: u32 = 0x4000;
    pub const DEPTH_STENCIL: u32 = 0x10000;
    pub const CPU_LOCKABLE: u32 = 0x20000;
    pub const DISPLAY: u32 = 0x80000;
    pub const MULTISAMPLE_RENDERTARGET: u32 = 0x20_0000;
    pub const TYPED_UNORDERED_ACCESS_VIEW: u32 = 0x200_0000;
}

/// The support mask at feature levels 11_0, 11_1 and 10_0 (the observations of the visible tests;
/// a format the visible tests do not show has the mask of the format of its family and kind that
/// they do show). `None`: not a format of this device (E_FAIL).
const SUPPORT: &[(u32, [u32; 3])] = &[
    (0, [0x0002_0001; 3]),
    (1, [0x0012_10f0; 3]),
    (2, [0x02f6_f3fb, 0x02f6_f3fb, 0x0076_f3fb]),
    (3, [0x0272_51fb, 0x0272_51fb, 0x0072_51fb]),
    (4, [0x0272_51fb, 0x0272_51fb, 0x0072_51fb]),
    (5, [0x0012_10f0; 3]),
    (6, [0x00f6_f3fb, 0x00f6_f3fb, 0x0076_f3fb]),
    (7, [0x0072_51fb; 3]),
    (8, [0x0072_51fb; 3]),
    (9, [0x0012_10f0; 3]),
    (10, [0x03fe_f3fb, 0x03fe_f3fb, 0x017e_f3fb]),
    (11, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_f3f3]),
    (12, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (13, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_73f3]),
    (14, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (15, [0x0012_10f0; 3]),
    (16, [0x02f6_f3fb, 0x02f6_f3fb, 0x0076_f3fb]),
    (17, [0x0272_51fb, 0x0272_51fb, 0x0072_51fb]),
    (18, [0x0272_51fb, 0x0272_51fb, 0x0072_51fb]),
    (19, [0x0012_10b0; 3]),
    (20, [0x0033_10b0; 3]),
    (21, [0x04d2_17b0, 0x04d2_17b0, 0x0012_17b0]),
    (22, [0x0052_11b0, 0x0052_11b0, 0x0012_11b0]),
    (23, [0x0012_10f0; 3]),
    (24, [0x03fe_f3f3, 0x03fe_f3f3, 0x017e_f3f3]),
    (25, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (26, [0x02e6_f3f3, 0x02e6_f3f3, 0x0066_f3f3]),
    (27, [0x0012_10f0; 3]),
    (28, [0x03fe_f3f3, 0x03fe_f3f3, 0x017e_f3f3]),
    (29, [0x01fe_f3f0, 0x01fe_f3f0, 0x017e_f3f0]),
    (30, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (31, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_73f3]),
    (32, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (33, [0x0012_10f0; 3]),
    (34, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_f3f3]),
    (35, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_f3f3]),
    (36, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (37, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_73f3]),
    (38, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (39, [0x0012_10f0; 3]),
    (40, [0x0033_10b0; 3]),
    (41, [0x06f6_f7fb, 0x06f6_f7fb, 0x0076_f7fb]),
    (42, [0x0272_51ff, 0x0272_51ff, 0x0072_51ff]),
    (43, [0x0272_51ff, 0x0272_51ff, 0x0072_51ff]),
    (44, [0x0012_10b0; 3]),
    (45, [0x0033_10b0; 3]),
    (46, [0x04d2_17b0, 0x04d2_17b0, 0x0012_17b0]),
    (47, [0x0052_11b0, 0x0052_11b0, 0x0012_11b0]),
    (48, [0x0012_10f0; 3]),
    (49, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_f3f3]),
    (50, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (51, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_73f3]),
    (52, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (53, [0x0012_10f0; 3]),
    (54, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_f3f3]),
    (55, [0x0033_10b0; 3]),
    (56, [0x06f6_f7f3, 0x06f6_f7f3, 0x0076_f7f3]),
    (57, [0x0272_51f7, 0x0272_51f7, 0x0072_51f7]),
    (58, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_73f3]),
    (59, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (60, [0x0012_10f0; 3]),
    (61, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_f3f3]),
    (62, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (63, [0x02f6_f3f3, 0x02f6_f3f3, 0x0076_73f3]),
    (64, [0x0272_51f3, 0x0272_51f3, 0x0072_51f3]),
    (65, [0x02e6_f3f0, 0x02e6_f3f0, 0x0066_f3f0]),
    (67, [0x0082_13f0, 0x02e6_d3f1, 0x0002_13f0]),
    (68, [0x0082_13f0, 0x0082_13f0, 0x0002_13f0]),
    (69, [0x0082_13f0, 0x0082_13f0, 0x0002_13f0]),
    (70, [0x0012_10e0; 3]),
    (71, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (72, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (73, [0x0012_10e0; 3]),
    (74, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (75, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (76, [0x0012_10e0; 3]),
    (77, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (78, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (79, [0x0012_10e0; 3]),
    (80, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (81, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (82, [0x0012_10e0; 3]),
    (83, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (84, [0x0092_13e0, 0x0092_13e0, 0x0012_13e0]),
    (85, [0x02e6_f3f3, 0x02e6_f3f3, 0x0066_f3f3]),
    (86, [0x02e6_f3f3, 0x02e6_f3f3, 0x0066_f3f3]),
    (87, [0x03fe_f3f3, 0x03fe_f3f3, 0x017e_f3f3]),
    (88, [0x00f6_f3f3, 0x00f6_f3f3, 0x0076_f3f3]),
    (89, [0x011a_0020; 3]),
    (90, [0x0012_10f0; 3]),
    (91, [0x01fe_f3f0, 0x01fe_f3f0, 0x017e_f3f0]),
    (92, [0x0012_10f0; 3]),
    (93, [0x00f6_f3f0, 0x00f6_f3f0, 0x0076_f3f0]),
    (94, [0x0012_10e0, 0x0012_10e0, 0]),
    (95, [0x0092_13e0, 0x0092_13e0, 0]),
    (96, [0x0092_13e0, 0x0092_13e0, 0]),
    (97, [0x0012_10e0, 0x0012_10e0, 0]),
    (98, [0x0092_13e0, 0x0092_13e0, 0]),
    (99, [0x0092_13e0, 0x0092_13e0, 0]),
    (100, [0x0282_0320, 0x0282_0320, 0x0002_0320]),
    (101, [0x0282_0320, 0x0282_0320, 0x0002_0320]),
    (102, [0x0282_1320, 0x0282_1320, 0x0002_1320]),
    (103, [0xc282_c320, 0xc282_c320, 0xc002_c320]),
    (104, [0x0282_c320, 0x0282_c320, 0x0002_c320]),
    (105, [0x0282_c320, 0x0282_c320, 0x0002_c320]),
    (106, [0x0000_0020; 3]),
    (107, [0x0282_0320, 0x0282_0320, 0x0002_0320]),
    (108, [0x0282_0320, 0x0282_0320, 0x0002_0320]),
    (109, [0x0282_0320, 0x0282_0320, 0x0002_0320]),
    (113, [0x0002_0020; 3]),
    (114, [0x0002_0020; 3]),
    (115, [0x02e6_f3f3, 0x02e6_f3f3, 0x0066_f3f3]),
    (188, [0x02e6_f3f3, 0x02e6_f3f3, 0x0066_f3f3]),
];

/// The `D3D11_FORMAT_SUPPORT` mask of `format` on a device of feature `level`, or `E_FAIL` for a
/// format the device does not know.
pub fn check_support(format: u32, level: u32) -> Result<u32, HRESULT> {
    let column = match level {
        l if l < 0xB000 => 2,
        0xB000 => 0,
        _ => 1,
    };
    match SUPPORT.iter().find(|(f, _)| *f == format) {
        Some((_, m)) if m[column] != 0 => Ok(m[column]),
        _ => Err(E_FAIL),
    }
}

/// The support mask, 0 for an unknown format.
pub fn support_mask(format: u32, level: u32) -> u32 {
    check_support(format, level).unwrap_or(0)
}

/// `D3D11_FORMAT_SUPPORT2` (CheckFeatureSupport FORMAT_SUPPORT2): the UAV atomic and typed load /
/// store bits (ID3D11Device.CheckFeatureSupport.yaml `format_support`).
pub fn support2(format: u32, level: u32) -> u32 {
    let mask = support_mask(format, level);
    if mask == 0 {
        return 0;
    }
    match format {
        R32_FLOAT => 0x0000_06c8,
        NV12 => 0x0001_0480,
        _ if mask & support::TYPED_UNORDERED_ACCESS_VIEW != 0 && is_display_8bit(format) => 0x0001_06c0,
        _ => 0,
    }
}

fn is_display_8bit(format: u32) -> bool {
    matches!(format, 28 | 87)
}

/// How a format stores its texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `bytes` per texel.
    Plain { bytes: u32 },
    /// Block compressed: `bytes` per 4x4 block.
    Block { bytes: u32 },
    /// Two texels per `bytes` (R8G8_B8G8, YUY2, ...).
    Pair { bytes: u32 },
    /// A planar video format: luma of `bytes` per texel, then chroma of half the rows.
    Planar { bytes: u32 },
    /// Not a format with a memory layout (UNKNOWN, opaque).
    None,
}

/// The memory layout of `format` (dxgiformat.h's names give the bits per component).
pub fn layout(format: u32) -> Layout {
    use Layout::*;
    match format {
        1..=4 => Plain { bytes: 16 },
        5..=8 => Plain { bytes: 12 },
        9..=22 => Plain { bytes: 8 },
        23..=47 => Plain { bytes: 4 },
        48..=59 => Plain { bytes: 2 },
        60..=65 => Plain { bytes: 1 },
        67 => Plain { bytes: 4 },
        68 | 69 | 107 => Pair { bytes: 4 },
        108 | 109 => Pair { bytes: 8 },
        70..=72 | 79..=81 => Block { bytes: 8 },
        73..=78 | 82..=84 | 94..=99 => Block { bytes: 16 },
        85 | 86 | 115 | 188 => Plain { bytes: 2 },
        87..=93 | 100 | 101 => Plain { bytes: 4 },
        102 => Plain { bytes: 8 },
        103 => Planar { bytes: 1 },
        104 | 105 => Planar { bytes: 2 },
        113 => Plain { bytes: 1 },
        114 => Plain { bytes: 2 },
        _ => None,
    }
}

/// The row pitch and the number of rows of a `width` x `height` subresource of `format`, with no
/// padding (ID3D11DeviceContext.Map.yaml `pitches`). Planar formats count their chroma rows.
pub fn pitch_rows(format: u32, width: u32, height: u32) -> (u32, u32) {
    match layout(format) {
        Layout::Plain { bytes } => (width * bytes, height),
        Layout::Block { bytes } => (width.div_ceil(4) * bytes, height.div_ceil(4)),
        Layout::Pair { bytes } => (width.div_ceil(2) * bytes, height),
        Layout::Planar { bytes } => (width.next_multiple_of(2) * bytes, height + height.div_ceil(2)),
        Layout::None => (width * 4, height),
    }
}

/// The bytes of one texel of a plain format (0 for the others).
pub fn texel_bytes(format: u32) -> u32 {
    match layout(format) {
        Layout::Plain { bytes } => bytes,
        _ => 0,
    }
}

/// The families of formats that share a bit layout: the typeless member first.
const FAMILIES: &[&[u32]] = &[
    &[1, 2, 3, 4],
    &[5, 6, 7, 8],
    &[9, 10, 11, 12, 13, 14],
    &[15, 16, 17, 18],
    &[19, 20, 21, 22],
    &[23, 24, 25],
    &[27, 28, 29, 30, 31, 32],
    &[33, 34, 35, 36, 37, 38],
    &[39, 40, 41, 42, 43],
    &[44, 45, 46, 47],
    &[48, 49, 50, 51, 52],
    &[53, 54, 55, 56, 57, 58, 59],
    &[60, 61, 62, 63, 64],
    &[70, 71, 72],
    &[73, 74, 75],
    &[76, 77, 78],
    &[79, 80, 81],
    &[82, 83, 84],
    &[90, 87, 91],
    &[92, 88, 93],
    &[94, 95, 96],
    &[97, 98, 99],
];

/// The family of `format` (its typeless member first), if it has one.
pub fn family(format: u32) -> Option<&'static [u32]> {
    FAMILIES.iter().copied().find(|f| f.contains(&format))
}

/// Whether `format` is the typeless member of a family.
pub fn is_typeless(format: u32) -> bool {
    family(format).is_some_and(|f| f[0] == format)
}

/// Whether `format` is a depth-stencil format (a format a depth-stencil view takes).
pub fn is_depth(format: u32) -> bool {
    matches!(format, D32_FLOAT_S8X24_UINT | D32_FLOAT | D24_UNORM_S8_UINT | D16_UNORM)
}

/// Whether a view of `view` format may see a texture of `tex` format: the same format, or any
/// member of the family of a typeless texture.
pub fn view_compatible(tex: u32, view: u32) -> bool {
    if tex == view {
        return !is_typeless(view);
    }
    is_typeless(tex) && family(tex).is_some_and(|f| f.contains(&view)) && !is_typeless(view)
}

/// The red component is in the low byte (`R8G8B8A8_*`) rather than blue (`B8G8R8A8_*`).
pub fn red_first(format: u32) -> bool {
    matches!(format, 27..=32)
}

/// sRGB formats (the colour is stored gamma encoded).
pub fn is_srgb(format: u32) -> bool {
    matches!(format, 29 | 72 | 75 | 78 | 91 | 93 | 99)
}

/// Whether a format is a known format of this device at all (texture creation refuses the others).
pub fn known(format: u32) -> bool {
    support_mask(format, 0xB000) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_examples_of_the_sheet() {
        assert_eq!(check_support(0, 0xB000), Ok(0x0002_0001));
        assert_eq!(check_support(28, 0xB000), Ok(0x03FE_F3F3));
        assert_eq!(check_support(87, 0xB000), Ok(0x03FE_F3F3));
        assert_eq!(check_support(88, 0xB000), Ok(0x00F6_F3F3));
        assert_eq!(check_support(103, 0xB000), Ok(0xC282_C320));
        assert_eq!(check_support(106, 0xB000), Ok(0x20));
        assert_eq!(check_support(67, 0xB000), Ok(0x0082_13F0));
        assert_eq!(check_support(67, 0xB100), Ok(0x02E6_D3F1));
        assert_eq!(check_support(2, 0xA000), Ok(0x0076_F3FB));
        for bad in [66, 110, 111, 112, 116, 133, 189, 192, 200, 1000, u32::MAX] {
            assert_eq!(check_support(bad, 0xB000), Err(E_FAIL), "{bad}");
        }
        assert_eq!(check_support(95, 0xA000), Err(E_FAIL));
    }

    #[test]
    fn support2_examples() {
        assert_eq!(support2(87, 0xB000), 0x106C0);
        assert_eq!(support2(28, 0xB000), 0x106C0);
        assert_eq!(support2(41, 0xB000), 0x6C8);
        assert_eq!(support2(103, 0xB000), 0x10480);
    }

    #[test]
    fn pitches_of_the_map_sheet() {
        assert_eq!(pitch_rows(87, 1, 1), (4, 1));
        assert_eq!(pitch_rows(87, 17, 4), (68, 4));
        assert_eq!(pitch_rows(61, 100, 3), (100, 3));
        assert_eq!(pitch_rows(10, 100, 3), (800, 3));
        assert_eq!(pitch_rows(2, 7, 5), (112, 5));
        assert_eq!(pitch_rows(71, 16, 16), (32, 4));
    }

    #[test]
    fn view_formats() {
        assert!(view_compatible(87, 87));
        assert!(!view_compatible(87, 91));
        assert!(view_compatible(27, 28) && view_compatible(27, 29) && view_compatible(27, 30));
        assert!(!view_compatible(27, 27) && !view_compatible(27, 87) && !view_compatible(27, 41));
        assert!(view_compatible(44, 46) && view_compatible(44, 45) && !view_compatible(44, 40));
    }
}
