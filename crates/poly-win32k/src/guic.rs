//! The third GUI slice's shared definitions (ADR 0172): DPI awareness
//! (`winuser.h` `DPI_AWARENESS_CONTEXT`, `windef.h` `DPI_AWARENESS`) and the
//! window state kept besides the first two slices'.

use alloc::vec::Vec;

/// Poly's internal awareness values (one per context).
pub const AWARE_UNAWARE: u8 = 0;
pub const AWARE_SYSTEM: u8 = 1;
pub const AWARE_PER_MONITOR: u8 = 2;
pub const AWARE_PER_MONITOR_V2: u8 = 3;
pub const AWARE_GDI_SCALED: u8 = 4;

/// `ERROR_INVALID_STATE` (`winerror.h`): `SetParent` across DPI kinds.
pub const ERROR_INVALID_STATE: u32 = 5023;

/// Which windows may be parent and child: unaware and GDI-scaled together,
/// the per-monitor kinds together, system-aware alone.
pub fn kind(a: u8) -> u8 {
    match a {
        AWARE_UNAWARE | AWARE_GDI_SCALED => 0,
        AWARE_SYSTEM => 1,
        _ => 2,
    }
}

/// `DPI_AWARENESS` (`windef.h`) of an awareness.
pub fn dpi_awareness(a: u8) -> i32 {
    match a {
        AWARE_SYSTEM => 1,
        AWARE_PER_MONITOR | AWARE_PER_MONITOR_V2 => 2,
        _ => 0,
    }
}

/// The awareness a `DPI_AWARENESS_CONTEXT` value names: the pseudo handles
/// `-1..=-5` and the values the real handles have (`GetThreadDpiAwarenessContext.yaml`).
pub fn context_awareness(h: i64) -> Option<u8> {
    // Real handles carry flag bits above the context: bit 31 is ignored.
    let h = if !(-5..0).contains(&h) { i64::from((h as u32) & 0x7FFF_FFFF) } else { h };
    Some(match h {
        -1 | 0x6010 => AWARE_UNAWARE,
        -2 | 0x6011 => AWARE_SYSTEM,
        -3 | 0x12 => AWARE_PER_MONITOR,
        -4 | 0x22 => AWARE_PER_MONITOR_V2,
        -5 | 0x4000_6010 => AWARE_GDI_SCALED,
        _ => return None,
    })
}

/// The real handle value of an awareness.
pub fn context_handle(a: u8) -> i64 {
    match a {
        AWARE_SYSTEM => 0x6011,
        AWARE_PER_MONITOR => 0x12,
        AWARE_PER_MONITOR_V2 => 0x22,
        AWARE_GDI_SCALED => 0x4000_6010,
        _ => 0x6010,
    }
}

/// The system colours `COLOR_*` 0..=30 (`winuser.h`) as `0x00BBGGRR`, the
/// oracle's light theme (`GetSysColor.yaml` `values`); index 25 is no
/// defined colour and reads 0.
pub const SYS_COLORS: [u32; 31] = [
    0x00C8_C8C8, // COLOR_SCROLLBAR
    0x0000_0000, // COLOR_BACKGROUND
    0x00D1_B499, // COLOR_ACTIVECAPTION
    0x00DB_CDBF, // COLOR_INACTIVECAPTION
    0x00F0_F0F0, // COLOR_MENU
    0x00FF_FFFF, // COLOR_WINDOW
    0x0064_6464, // COLOR_WINDOWFRAME
    0x0000_0000, // COLOR_MENUTEXT
    0x0000_0000, // COLOR_WINDOWTEXT
    0x0000_0000, // COLOR_CAPTIONTEXT
    0x00B4_B4B4, // COLOR_ACTIVEBORDER
    0x00FC_F7F4, // COLOR_INACTIVEBORDER
    0x00AB_ABAB, // COLOR_APPWORKSPACE
    0x00D4_7800, // COLOR_HIGHLIGHT
    0x00FF_FFFF, // COLOR_HIGHLIGHTTEXT
    0x00F0_F0F0, // COLOR_BTNFACE
    0x00A0_A0A0, // COLOR_BTNSHADOW
    0x006D_6D6D, // COLOR_GRAYTEXT
    0x0000_0000, // COLOR_BTNTEXT
    0x0000_0000, // COLOR_INACTIVECAPTIONTEXT
    0x00FF_FFFF, // COLOR_BTNHIGHLIGHT
    0x0069_6969, // COLOR_3DDKSHADOW
    0x00E3_E3E3, // COLOR_3DLIGHT
    0x0000_0000, // COLOR_INFOTEXT
    0x00E1_FFFF, // COLOR_INFOBK
    0x0000_0000, // 25: not a defined colour
    0x00CC_6600, // COLOR_HOTLIGHT
    0x00EA_D1B9, // COLOR_GRADIENTACTIVECAPTION
    0x00F2_E4D7, // COLOR_GRADIENTINACTIVECAPTION
    0x00D4_7800, // COLOR_MENUHILIGHT
    0x00F0_F0F0, // COLOR_MENUBAR
];

/// `GetSysColor(index)`: the colour, or `None` for an index that is no
/// system colour (the caller fails with ERROR_INVALID_PARAMETER).
pub fn sys_color(index: i32) -> Option<u32> {
    usize::try_from(index).ok().and_then(|i| SYS_COLORS.get(i)).copied()
}

/// `DWMWA_EXTENDED_FRAME_BOUNDS`: how far the visible frame lies inside
/// the window rectangle at the left, right and bottom (the top is kept):
/// 7 for a window with a caption, 6 for a popup with only a thick frame,
/// 0 for a plain popup (`DwmGetWindowAttribute.yaml` `frame_bounds`).
pub fn dwm_frame_inset(style: u32) -> i32 {
    use crate::metrics::{WS_CAPTION, WS_POPUP, WS_THICKFRAME};
    if style & WS_CAPTION == WS_CAPTION {
        7
    } else if style & WS_POPUP != 0 && style & WS_THICKFRAME != 0 {
        6
    } else {
        0
    }
}

/// `DWMWA_CAPTION_BUTTON_BOUNDS` of a window `width` wide, relative to its
/// top-left (`DwmGetWindowAttribute.yaml` `caption_buttons`): with a
/// caption the buttons end 7 pixels before the right edge and are 30 high:
/// 146 wide with a minimize or maximize box and the close button, 49 with
/// the system menu alone or for a tool window, nothing without a system
/// menu; a popup with only a thick frame has an empty rectangle 6 pixels
/// in, 29 high; a plain popup has none.
pub fn dwm_caption_buttons(style: u32, ex_style: u32, width: i32) -> [i32; 4] {
    use crate::metrics::{WS_CAPTION, WS_EX_TOOLWINDOW, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME};
    const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
    if style & WS_CAPTION == WS_CAPTION {
        let right = width - 7;
        let buttons = if style & WS_SYSMENU == 0 {
            0
        } else if style & (WS_MINIMIZEBOX | WS_MAXIMIZEBOX) != 0 && ex_style & WS_EX_TOOLWINDOW == 0 {
            146
        } else {
            49
        };
        [right - buttons, 0, right, 30]
    } else if style & WS_THICKFRAME != 0 {
        [width - 6, 0, width - 6, 29]
    } else {
        [0, 0, 0, 0]
    }
}

/// `WS_EX_LAYERED` (`winuser.h`).
pub const WS_EX_LAYERED: u32 = 0x0008_0000;

/// `ULW_COLORKEY`, `ULW_ALPHA`, `ULW_OPAQUE` (`winuser.h`).
pub const ULW_COLORKEY: u32 = 1;
pub const ULW_ALPHA: u32 = 2;
pub const ULW_OPAQUE: u32 = 4;

/// A window's layered attributes (`SetLayeredWindowAttributes.yaml`):
/// what the last call stored, whether any call has (`attrs`), and whether
/// `UpdateLayeredWindow` has been used (`update`), with how its content is
/// shown: the `ULW_*` flags, the colour key, the blend's
/// `SourceConstantAlpha` and whether its `AlphaFormat` had `AC_SRC_ALPHA`
/// (`UpdateLayeredWindow.yaml`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layered {
    pub key: u32,
    pub alpha: u8,
    pub flags: u32,
    pub attrs: bool,
    pub update: bool,
    pub ulw_flags: u32,
    pub ulw_key: u32,
    pub ulw_alpha: u8,
    pub ulw_src_alpha: bool,
}

impl Layered {
    /// What the screen shows where a window given its content by
    /// `UpdateLayeredWindow` has the pixel `px` (`B, G, R, A`) over
    /// `below` (a `COLORREF`), by the rules of `UpdateLayeredWindow.yaml`:
    /// a pixel equal to the colour key shows what is below; with
    /// `ULW_ALPHA` and `AC_SRC_ALPHA` the pixel is premultiplied (its
    /// colour, scaled by the constant alpha, is added to what is below
    /// scaled by the rest of its alpha, saturating); with `ULW_ALPHA`
    /// alone the bitmap's alpha is ignored and the constant alpha blends;
    /// `ULW_OPAQUE` keeps opaque pixels and still composes a pixel's own
    /// alpha; otherwise the pixel shows as it is.
    pub fn ulw_pixel(&self, px: [u8; 4], below: u32) -> u32 {
        let (b, g, r, a) = (u32::from(px[0]), u32::from(px[1]), u32::from(px[2]), u32::from(px[3]));
        let own = r | g << 8 | b << 16;
        if self.ulw_flags & ULW_COLORKEY != 0 && own == self.ulw_key & 0x00FF_FFFF {
            return below;
        }
        let under = |sh: u32| (below >> sh) & 255;
        let each = |f: &dyn Fn(u32, u32) -> u32| f(r, under(0)) | f(g, under(8)) << 8 | f(b, under(16)) << 16;
        if self.ulw_flags & ULW_ALPHA != 0 {
            let sca = u32::from(self.ulw_alpha);
            if self.ulw_src_alpha {
                let a = a * sca / 255;
                each(&|s, d| (s * sca / 255 + d * (255 - a) / 255).min(255))
            } else {
                each(&|s, d| (s * sca + d * (255 - sca)) / 255)
            }
        } else if self.ulw_flags & ULW_OPAQUE != 0 {
            each(&|s, d| (s + d * (255 - a) / 255).min(255))
        } else {
            own
        }
    }

    /// Whether a hit on the pixel `px` of an `UpdateLayeredWindow`-ed
    /// window goes to what is below: with per-pixel alpha, where the
    /// pixel's alpha is 0 (`UpdateLayeredWindow.yaml` `hit_testing`).
    pub fn ulw_hit_passes(&self, px: [u8; 4]) -> bool {
        self.update && !self.attrs && self.ulw_flags & ULW_ALPHA != 0 && self.ulw_src_alpha && px[3] == 0
    }
}

/// The content `UpdateLayeredWindow` gives a window of `cx` x `cy` from a
/// `bw` x `bh` bitmap (`px`: `B, G, R, A` rows, top-down): the part of the
/// bitmap from (`sx`, `sy`) on (`UpdateLayeredWindow.yaml` `source_point`);
/// what lies outside the bitmap is empty (alpha 0). None when there is
/// nothing to show or it is more than `max` bytes.
#[allow(clippy::too_many_arguments)]
pub fn ulw_content(px: &[u8], bw: i32, bh: i32, sx: i32, sy: i32, cx: i32, cy: i32, max: usize) -> Option<Vec<u8>> {
    if cx <= 0 || cy <= 0 || bw <= 0 || bh <= 0 {
        return None;
    }
    let size = (cx as usize).checked_mul(cy as usize)?.checked_mul(4)?;
    if size > max || px.len() < (bw as usize).checked_mul(bh as usize)?.checked_mul(4)? {
        return None;
    }
    let mut out = alloc::vec![0u8; size];
    for y in 0..cy {
        let from_y = i64::from(sy) + i64::from(y);
        if from_y < 0 || from_y >= i64::from(bh) {
            continue;
        }
        for x in 0..cx {
            let from_x = i64::from(sx) + i64::from(x);
            if from_x < 0 || from_x >= i64::from(bw) {
                continue;
            }
            let s = (from_y as usize * bw as usize + from_x as usize) * 4;
            let d = (y as usize * cx as usize + x as usize) * 4;
            out[d..d + 4].copy_from_slice(&px[s..s + 4]);
        }
    }
    Some(out)
}

/// The extended style a `SetWindowLong` leaves: the layered bit is refused
/// for a child window, and removing it forgets the attributes.
pub fn filter_ex_style(style: u32, old: u32, new: u32, lay: &mut Layered) -> u32 {
    let mut new = new;
    if style & 0x4000_0000 != 0 {
        new &= !WS_EX_LAYERED;
    }
    if (old ^ new) & WS_EX_LAYERED != 0 {
        *lay = Layered::default();
    }
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts_map_both_ways() {
        for h in [-1i64, -2, -3, -4, -5] {
            let a = context_awareness(h).unwrap();
            assert_eq!(context_awareness(context_handle(a)), Some(a));
        }
        assert_eq!(context_awareness(0xFFFF_FFFF_8000_6010u64 as i64), Some(AWARE_UNAWARE));
        for bad in [0i64, 1, 2, 3, -6, -7, -16, 16, 0x6012, 0x6013, 0x1234, 0x7FFF_FFFF, 0x4000_6011] {
            assert_eq!(context_awareness(bad), None, "{bad:#x}");
        }
        assert_eq!(dpi_awareness(AWARE_PER_MONITOR_V2), 2);
    }

    /// UpdateLayeredWindow.yaml `per_pixel_alpha`, `constant_alpha`,
    /// `colorkey_and_opaque`, `hit_testing`: pixels over the blue popup
    /// (the sheet prints colours as 0xRRGGBB; a COLORREF is 0x00BBGGRR).
    #[test]
    fn update_layered_window_pixels_compose_as_the_sheet_says() {
        const BLUE: u32 = 0x00FF_0000;
        // 0xAARRGGBB -> B, G, R, A.
        let px = |argb: u32| [argb as u8, (argb >> 8) as u8, (argb >> 16) as u8, (argb >> 24) as u8];
        // 0xRRGGBB as the sheet prints it.
        let shown = |c: u32| (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF;
        let mode = |flags, alpha, src_alpha, key| Layered {
            update: true,
            ulw_flags: flags,
            ulw_alpha: alpha,
            ulw_src_alpha: src_alpha,
            ulw_key: key,
            ..Layered::default()
        };
        let per_pixel = mode(ULW_ALPHA, 255, true, 0);
        for (argb, want) in [
            (0xFF00_FF00u32, 0x00FF00),
            (0x8080_0000, 0x80007F),
            (0x80FF_0000, 0xFF007F),
            (0x00FF_0000, 0xFF00FF),
            (0x0000_0000, 0x0000FF),
        ] {
            assert_eq!(shown(per_pixel.ulw_pixel(px(argb), BLUE)), want, "{argb:#x}");
        }
        let half = mode(ULW_ALPHA, 128, true, 0);
        assert_eq!(shown(half.ulw_pixel(px(0xFF00_FF00), BLUE)), 0x00807F);
        assert_eq!(shown(half.ulw_pixel(px(0x8080_0000), BLUE)), 0x4000BF);
        // Without AC_SRC_ALPHA the bitmap's alpha is ignored.
        let flat = mode(ULW_ALPHA, 255, false, 0);
        assert_eq!(shown(flat.ulw_pixel(px(0xFF00_FF00), BLUE)), 0x00FF00);
        assert_eq!(shown(flat.ulw_pixel(px(0x8080_0000), BLUE)), 0x800000);
        assert_eq!(shown(mode(ULW_ALPHA, 128, false, 0).ulw_pixel(px(0x8080_0000), BLUE)), 0x40007F);
        // A colour key (a COLORREF) hides exactly its colour.
        let green_key = mode(ULW_COLORKEY, 0, false, 0x0000_FF00);
        assert_eq!(shown(green_key.ulw_pixel(px(0xFF00_FF00), BLUE)), 0x0000FF);
        assert_eq!(shown(green_key.ulw_pixel(px(0xFFFF_0000), BLUE)), 0xFF0000);
        let red_key = mode(ULW_COLORKEY, 0, false, 0x0000_00FF);
        assert_eq!(shown(red_key.ulw_pixel(px(0xFFFF_0000), BLUE)), 0x0000FF);
        assert_eq!(shown(red_key.ulw_pixel(px(0xFF00_FF00), BLUE)), 0x00FF00);
        // ULW_OPAQUE: opaque pixels as they are, a half-transparent one composed.
        let opaque = mode(ULW_OPAQUE, 0, false, 0);
        assert_eq!(shown(opaque.ulw_pixel(px(0xFF00_FF00), BLUE)), 0x00FF00);
        assert_eq!(shown(opaque.ulw_pixel(px(0x8080_0000), BLUE)), 0x80007F);
        // Hits pass where the alpha is 0, also when the pixel has a colour.
        assert!(per_pixel.ulw_hit_passes(px(0)));
        assert!(per_pixel.ulw_hit_passes(px(0x00FF_0000)));
        assert!(!per_pixel.ulw_hit_passes(px(0xFF00_FF00)));
        assert!(!flat.ulw_hit_passes(px(0)));
        assert!(!Layered { attrs: true, ..per_pixel }.ulw_hit_passes(px(0)));
    }

    /// UpdateLayeredWindow.yaml `source_point`: a 40x30 window from the
    /// source point (30, 0) of a bitmap whose x < 50 is green and the rest
    /// red is green for x 0..19 and red for x 20..39.
    #[test]
    fn update_layered_window_content_starts_at_the_source_point() {
        let (bw, bh) = (100, 30);
        let green = [0u8, 0xFF, 0, 0xFF];
        let red = [0u8, 0, 0xFF, 0xFF];
        let px: alloc::vec::Vec<u8> = (0..bw * bh).flat_map(|k| if k % bw < 50 { green } else { red }).collect();
        let c = ulw_content(&px, bw, bh, 30, 0, 40, 30, 1 << 20).unwrap();
        assert_eq!(c.len(), 40 * 30 * 4);
        for y in [0usize, 29] {
            for x in 0..40usize {
                let at = (y * 40 + x) * 4;
                assert_eq!(c[at..at + 4], if x < 20 { green } else { red }, "{x},{y}");
            }
        }
        // Outside the bitmap there is nothing; a size larger than the bitmap is fine.
        let c = ulw_content(&px, bw, bh, 90, -1, 20, 3, 1 << 20).unwrap();
        assert_eq!(c[..4], [0; 4]);
        assert_eq!(c[20 * 4..20 * 4 + 4], red);
        assert_eq!(c[(20 + 10) * 4..(20 + 10) * 4 + 4], [0; 4]);
        assert!(ulw_content(&px, bw, bh, 1000, 1000, 10, 10, 1 << 20).unwrap().iter().all(|&b| b == 0));
        assert_eq!(ulw_content(&px, bw, bh, 0, 0, 0, 10, 1 << 20), None);
        assert_eq!(ulw_content(&px, bw, bh, 0, 0, 40, 30, 100), None);
        assert_eq!(ulw_content(&px[..8], bw, bh, 0, 0, 4, 4, 1 << 20), None);
    }

    /// DwmGetWindowAttribute.yaml `frame_bounds`, `caption_buttons`.
    #[test]
    fn dwm_bounds_follow_the_style() {
        use crate::metrics::*;
        const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
        let overlapped = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        // Frame bounds: every window with a caption loses 7.
        for s in [overlapped, WS_CAPTION, WS_CAPTION | WS_THICKFRAME, WS_CAPTION | WS_SYSMENU, WS_POPUP | WS_CAPTION] {
            assert_eq!(dwm_frame_inset(s), 7, "{s:#x}");
        }
        assert_eq!(dwm_frame_inset(WS_POPUP | WS_THICKFRAME), 6);
        assert_eq!(dwm_frame_inset(WS_POPUP), 0);
        assert_eq!(dwm_frame_inset(WS_POPUP | WS_BORDER), 0);
        // Caption buttons of a window 300 wide.
        assert_eq!(dwm_caption_buttons(overlapped, 0, 300), [147, 0, 293, 30]);
        assert_eq!(dwm_caption_buttons(WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX, 0, 300), [147, 0, 293, 30]);
        assert_eq!(dwm_caption_buttons(WS_CAPTION | WS_SYSMENU | WS_MAXIMIZEBOX, 0, 300), [147, 0, 293, 30]);
        assert_eq!(dwm_caption_buttons(WS_CAPTION | WS_SYSMENU, 0, 300), [244, 0, 293, 30]);
        assert_eq!(dwm_caption_buttons(overlapped, WS_EX_TOOLWINDOW, 300), [244, 0, 293, 30]);
        assert_eq!(dwm_caption_buttons(WS_CAPTION, 0, 300), [293, 0, 293, 30]);
        assert_eq!(dwm_caption_buttons(WS_CAPTION | WS_MINIMIZEBOX | WS_MAXIMIZEBOX, 0, 300), [293, 0, 293, 30]);
        assert_eq!(dwm_caption_buttons(WS_POPUP, 0, 300), [0, 0, 0, 0]);
        assert_eq!(dwm_caption_buttons(WS_POPUP | WS_BORDER, 0, 300), [0, 0, 0, 0]);
        assert_eq!(dwm_caption_buttons(WS_POPUP | WS_THICKFRAME, 0, 300), [294, 0, 294, 29]);
    }

    /// GetSysColor.yaml `values`, `table`, `invalid`.
    #[test]
    fn system_colours_are_the_sheets() {
        let sheet: [(i32, u32); 31] = [
            (0, 0xc8c8c8),
            (1, 0x000000),
            (2, 0xd1b499),
            (3, 0xdbcdbf),
            (4, 0xf0f0f0),
            (5, 0xffffff),
            (6, 0x646464),
            (7, 0),
            (8, 0),
            (9, 0),
            (10, 0xb4b4b4),
            (11, 0xfcf7f4),
            (12, 0xababab),
            (13, 0xd47800),
            (14, 0xffffff),
            (15, 0xf0f0f0),
            (16, 0xa0a0a0),
            (17, 0x6d6d6d),
            (18, 0),
            (19, 0),
            (20, 0xffffff),
            (21, 0x696969),
            (22, 0xe3e3e3),
            (23, 0),
            (24, 0xe1ffff),
            (25, 0),
            (26, 0xcc6600),
            (27, 0xead1b9),
            (28, 0xf2e4d7),
            (29, 0xd47800),
            (30, 0xf0f0f0),
        ];
        for (i, c) in sheet {
            assert_eq!(sys_color(i), Some(c), "COLOR index {i}");
        }
        for bad in [-1, 31, 32, 33, 100, 0x1000, i32::MAX, i32::MIN] {
            assert_eq!(sys_color(bad), None, "{bad}");
        }
        assert_eq!(kind(AWARE_GDI_SCALED), kind(AWARE_UNAWARE));
        assert_ne!(kind(AWARE_SYSTEM), kind(AWARE_PER_MONITOR));
    }
}
