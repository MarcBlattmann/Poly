//! `SystemParametersInfoW`'s queries (`SPI_GET*`, ADR 0155 batch 4): what a
//! default Windows 10 desktop session reports to a program that asks, as
//! the bytes to write to `pvParam`.
//!
//! Chrome asks for a handful of them to size its UI (the non-client
//! metrics and fonts, wheel scrolling, the caret, animation and font
//! smoothing switches, accessibility structures). Nothing here can change:
//! the `SPI_SET*` actions are not served. Values are the defaults of a
//! fresh Windows 10 installation at 96 DPI as Microsoft Learn documents
//! them where it gives defaults (wheel scroll lines 3, hover time 400 ms,
//! caret width 1, ...); the constants are `winuser.h`'s.

use alloc::vec::Vec;

/// `ERROR_INVALID_PARAMETER` (`winerror.h`).
pub const ERROR_INVALID_PARAMETER: u32 = 87;

pub const SPI_GETBEEP: u32 = 0x0001;
pub const SPI_GETMOUSE: u32 = 0x0003;
pub const SPI_GETBORDER: u32 = 0x0005;
pub const SPI_GETKEYBOARDSPEED: u32 = 0x000A;
pub const SPI_GETSCREENSAVETIMEOUT: u32 = 0x000E;
pub const SPI_GETSCREENSAVEACTIVE: u32 = 0x0010;
pub const SPI_GETGRIDGRANULARITY: u32 = 0x0012;
pub const SPI_GETKEYBOARDDELAY: u32 = 0x0016;
pub const SPI_ICONHORIZONTALSPACING: u32 = 0x000D;
pub const SPI_ICONVERTICALSPACING: u32 = 0x0018;
pub const SPI_GETICONTITLEWRAP: u32 = 0x0019;
pub const SPI_GETMENUDROPALIGNMENT: u32 = 0x001B;
pub const SPI_GETICONTITLELOGFONT: u32 = 0x001F;
pub const SPI_GETDRAGFULLWINDOWS: u32 = 0x0026;
pub const SPI_GETNONCLIENTMETRICS: u32 = 0x0029;
pub const SPI_GETMINIMIZEDMETRICS: u32 = 0x002B;
pub const SPI_GETICONMETRICS: u32 = 0x002D;
pub const SPI_GETWORKAREA: u32 = 0x0030;
pub const SPI_GETFILTERKEYS: u32 = 0x0032;
pub const SPI_GETTOGGLEKEYS: u32 = 0x0034;
pub const SPI_GETMOUSEKEYS: u32 = 0x0036;
pub const SPI_GETSHOWSOUNDS: u32 = 0x0038;
pub const SPI_GETSTICKYKEYS: u32 = 0x003A;
pub const SPI_GETACCESSTIMEOUT: u32 = 0x003C;
pub const SPI_GETHIGHCONTRAST: u32 = 0x0042;
pub const SPI_GETKEYBOARDPREF: u32 = 0x0044;
pub const SPI_GETSCREENREADER: u32 = 0x0046;
pub const SPI_GETANIMATION: u32 = 0x0048;
pub const SPI_GETFONTSMOOTHING: u32 = 0x004A;
pub const SPI_GETSNAPTODEFBUTTON: u32 = 0x005F;
pub const SPI_GETMOUSEHOVERWIDTH: u32 = 0x0062;
pub const SPI_GETMOUSEHOVERHEIGHT: u32 = 0x0064;
pub const SPI_GETMOUSEHOVERTIME: u32 = 0x0066;
pub const SPI_GETWHEELSCROLLLINES: u32 = 0x0068;
pub const SPI_GETMENUSHOWDELAY: u32 = 0x006A;
pub const SPI_GETWHEELSCROLLCHARS: u32 = 0x006C;
pub const SPI_GETSHOWIMEUI: u32 = 0x006E;
pub const SPI_GETMOUSESPEED: u32 = 0x0070;
pub const SPI_GETSCREENSAVERRUNNING: u32 = 0x0072;
pub const SPI_GETDESKWALLPAPER: u32 = 0x0073;
pub const SPI_GETACTIVEWINDOWTRACKING: u32 = 0x1000;
pub const SPI_GETMENUANIMATION: u32 = 0x1002;
pub const SPI_GETCOMBOBOXANIMATION: u32 = 0x1004;
pub const SPI_GETLISTBOXSMOOTHSCROLLING: u32 = 0x1006;
pub const SPI_GETGRADIENTCAPTIONS: u32 = 0x1008;
pub const SPI_GETKEYBOARDCUES: u32 = 0x100A;
pub const SPI_GETACTIVEWNDTRKZORDER: u32 = 0x100C;
pub const SPI_GETHOTTRACKING: u32 = 0x100E;
pub const SPI_GETMENUFADE: u32 = 0x1012;
pub const SPI_GETSELECTIONFADE: u32 = 0x1014;
pub const SPI_GETTOOLTIPANIMATION: u32 = 0x1016;
pub const SPI_GETTOOLTIPFADE: u32 = 0x1018;
pub const SPI_GETCURSORSHADOW: u32 = 0x101A;
pub const SPI_GETMOUSESONAR: u32 = 0x101C;
pub const SPI_GETMOUSECLICKLOCK: u32 = 0x101E;
pub const SPI_GETMOUSEVANISH: u32 = 0x1020;
pub const SPI_GETFLATMENU: u32 = 0x1022;
pub const SPI_GETDROPSHADOW: u32 = 0x1024;
pub const SPI_GETBLOCKSENDINPUTRESETS: u32 = 0x1026;
pub const SPI_GETUIEFFECTS: u32 = 0x103E;
pub const SPI_GETDISABLEOVERLAPPEDCONTENT: u32 = 0x1040;
pub const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
pub const SPI_GETCLEARTYPE: u32 = 0x1048;
pub const SPI_GETSPEECHRECOGNITION: u32 = 0x104A;
pub const SPI_GETFOREGROUNDLOCKTIMEOUT: u32 = 0x2000;
pub const SPI_GETACTIVEWNDTRKTIMEOUT: u32 = 0x2002;
pub const SPI_GETFOREGROUNDFLASHCOUNT: u32 = 0x2004;
pub const SPI_GETCARETWIDTH: u32 = 0x2006;
pub const SPI_GETMOUSECLICKLOCKTIME: u32 = 0x2008;
pub const SPI_GETFONTSMOOTHINGTYPE: u32 = 0x200A;
pub const SPI_GETFONTSMOOTHINGCONTRAST: u32 = 0x200C;
pub const SPI_GETFOCUSBORDERWIDTH: u32 = 0x200E;
pub const SPI_GETFOCUSBORDERHEIGHT: u32 = 0x2010;
pub const SPI_GETFONTSMOOTHINGORIENTATION: u32 = 0x2012;
pub const SPI_GETMESSAGEDURATION: u32 = 0x2016;

/// What a query answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Write these bytes to `pvParam`.
    Bytes(Vec<u8>),
    /// Fail with this Win32 error.
    Error(u32),
    /// Not a query this module knows.
    Unknown,
}

/// Whether `pvParam` of this action begins with the structure's `cbSize`
/// (so the caller reads it for [`query`]).
pub fn has_cb_size(action: u32) -> bool {
    matches!(
        action,
        SPI_GETNONCLIENTMETRICS
            | SPI_GETMINIMIZEDMETRICS
            | SPI_GETICONMETRICS
            | SPI_GETFILTERKEYS
            | SPI_GETTOGGLEKEYS
            | SPI_GETMOUSEKEYS
            | SPI_GETSTICKYKEYS
            | SPI_GETACCESSTIMEOUT
            | SPI_GETHIGHCONTRAST
            | SPI_GETANIMATION
    )
}

fn word(v: u32) -> Outcome {
    Outcome::Bytes(v.to_le_bytes().to_vec())
}

/// A `LOGFONTW` (92 bytes): Segoe UI, 9 points at 96 DPI (-12 pixels),
/// regular weight, ClearType quality — the font Windows 10 gives menus,
/// captions, status bars and message boxes.
pub fn ui_font() -> [u8; 92] {
    let mut f = [0u8; 92];
    f[0..4].copy_from_slice(&(-12i32).to_le_bytes()); // lfHeight
    f[16..20].copy_from_slice(&400i32.to_le_bytes()); // lfWeight FW_NORMAL
    f[23] = 1; // lfCharSet DEFAULT_CHARSET
    f[26] = 5; // lfQuality CLEARTYPE_QUALITY
    f[27] = 0x22; // lfPitchAndFamily FF_SWISS | VARIABLE_PITCH
    for (i, c) in "Segoe UI".encode_utf16().enumerate() {
        f[28 + i * 2..30 + i * 2].copy_from_slice(&c.to_le_bytes());
    }
    f
}

fn put(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// `NONCLIENTMETRICSW` (504 bytes, 500 before Vista's padded border).
fn non_client_metrics(cb: u32) -> Outcome {
    if cb != 504 && cb != 500 {
        return Outcome::Error(ERROR_INVALID_PARAMETER);
    }
    let mut o = Vec::with_capacity(504);
    put(&mut o, cb as i32);
    for v in [1, 17, 17, 22, 22] {
        // iBorderWidth, iScrollWidth, iScrollHeight, iCaptionWidth, iCaptionHeight
        put(&mut o, v);
    }
    o.extend_from_slice(&ui_font()); // lfCaptionFont
    put(&mut o, 22); // iSmCaptionWidth
    put(&mut o, 22); // iSmCaptionHeight
    o.extend_from_slice(&ui_font()); // lfSmCaptionFont
    put(&mut o, 19); // iMenuWidth
    put(&mut o, 19); // iMenuHeight
    for _ in 0..3 {
        o.extend_from_slice(&ui_font()); // lfMenuFont, lfStatusFont, lfMessageFont
    }
    if cb == 504 {
        put(&mut o, 4); // iPaddedBorderWidth (SM_CXPADDEDBORDER)
    }
    Outcome::Bytes(o)
}

/// A structure that is `cbSize` and zeros: `size` bytes.
fn zeroed(cb: u32, size: u32, flags: Option<(usize, u32)>) -> Outcome {
    if cb != size {
        return Outcome::Error(ERROR_INVALID_PARAMETER);
    }
    let mut o = alloc::vec![0u8; size as usize];
    o[0..4].copy_from_slice(&size.to_le_bytes());
    if let Some((at, v)) = flags {
        o[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    Outcome::Bytes(o)
}

/// Answer `SPI_GET*` `action`. `ui_param` is `uiParam`, `cb_size` the first
/// four bytes of `pvParam` when [`has_cb_size`] (else 0), `work` the work
/// area (left, top, right, bottom).
pub fn query(action: u32, ui_param: u32, cb_size: u32, work: [i32; 4]) -> Outcome {
    match action {
        SPI_GETBEEP | SPI_GETFONTSMOOTHING | SPI_GETDRAGFULLWINDOWS | SPI_GETICONTITLEWRAP | SPI_GETSHOWIMEUI => {
            word(1)
        }
        SPI_GETMENUANIMATION
        | SPI_GETCOMBOBOXANIMATION
        | SPI_GETLISTBOXSMOOTHSCROLLING
        | SPI_GETGRADIENTCAPTIONS
        | SPI_GETHOTTRACKING
        | SPI_GETMENUFADE
        | SPI_GETSELECTIONFADE
        | SPI_GETTOOLTIPANIMATION
        | SPI_GETTOOLTIPFADE
        | SPI_GETCURSORSHADOW
        | SPI_GETFLATMENU
        | SPI_GETDROPSHADOW
        | SPI_GETBLOCKSENDINPUTRESETS
        | SPI_GETUIEFFECTS
        | SPI_GETCLIENTAREAANIMATION
        | SPI_GETCLEARTYPE => word(1),
        SPI_GETSCREENSAVEACTIVE
        | SPI_GETMENUDROPALIGNMENT
        | SPI_GETSHOWSOUNDS
        | SPI_GETKEYBOARDPREF
        | SPI_GETSCREENREADER
        | SPI_GETSNAPTODEFBUTTON
        | SPI_GETSCREENSAVERRUNNING
        | SPI_GETACTIVEWINDOWTRACKING
        | SPI_GETKEYBOARDCUES
        | SPI_GETACTIVEWNDTRKZORDER
        | SPI_GETMOUSESONAR
        | SPI_GETMOUSECLICKLOCK
        | SPI_GETMOUSEVANISH
        | SPI_GETDISABLEOVERLAPPEDCONTENT
        | SPI_GETSPEECHRECOGNITION
        | SPI_GETFOREGROUNDLOCKTIMEOUT
        | SPI_GETACTIVEWNDTRKTIMEOUT
        | SPI_GETGRIDGRANULARITY => word(0),
        SPI_GETBORDER => word(1),
        SPI_GETKEYBOARDSPEED => word(31),
        SPI_GETKEYBOARDDELAY => word(1),
        SPI_GETSCREENSAVETIMEOUT => word(600),
        SPI_ICONHORIZONTALSPACING | SPI_ICONVERTICALSPACING => word(75),
        SPI_GETMOUSEHOVERWIDTH | SPI_GETMOUSEHOVERHEIGHT => word(4),
        SPI_GETMOUSEHOVERTIME | SPI_GETMENUSHOWDELAY => word(400),
        SPI_GETWHEELSCROLLLINES | SPI_GETWHEELSCROLLCHARS | SPI_GETFOREGROUNDFLASHCOUNT => word(3),
        SPI_GETMOUSESPEED => word(10),
        SPI_GETCARETWIDTH | SPI_GETFOCUSBORDERWIDTH | SPI_GETFOCUSBORDERHEIGHT => word(1),
        SPI_GETMOUSECLICKLOCKTIME => word(1200),
        SPI_GETFONTSMOOTHINGTYPE => word(2), // FE_FONTSMOOTHINGCLEARTYPE
        SPI_GETFONTSMOOTHINGCONTRAST => word(1400),
        SPI_GETFONTSMOOTHINGORIENTATION => word(1), // FE_FONTSMOOTHINGORIENTATIONRGB
        SPI_GETMESSAGEDURATION => word(5),
        SPI_GETMOUSE => {
            // Threshold 1, threshold 2, acceleration: 6, 10, 1.
            let mut o = Vec::with_capacity(12);
            for v in [6, 10, 1] {
                put(&mut o, v);
            }
            Outcome::Bytes(o)
        }
        SPI_GETWORKAREA => {
            let mut o = Vec::with_capacity(16);
            for v in work {
                put(&mut o, v);
            }
            Outcome::Bytes(o)
        }
        SPI_GETDESKWALLPAPER => {
            // No wallpaper: an empty path (uiParam is the buffer's size in characters).
            if ui_param == 0 { Outcome::Error(ERROR_INVALID_PARAMETER) } else { Outcome::Bytes(alloc::vec![0, 0]) }
        }
        SPI_GETICONTITLELOGFONT => {
            if ui_param != 92 {
                Outcome::Error(ERROR_INVALID_PARAMETER)
            } else {
                Outcome::Bytes(ui_font().to_vec())
            }
        }
        SPI_GETNONCLIENTMETRICS => non_client_metrics(cb_size),
        SPI_GETICONMETRICS => {
            if cb_size != 108 {
                return Outcome::Error(ERROR_INVALID_PARAMETER);
            }
            let mut o = Vec::with_capacity(108);
            for v in [108, 75, 75, 1] {
                put(&mut o, v);
            }
            o.extend_from_slice(&ui_font());
            Outcome::Bytes(o)
        }
        SPI_GETMINIMIZEDMETRICS => {
            if cb_size != 20 {
                return Outcome::Error(ERROR_INVALID_PARAMETER);
            }
            let mut o = Vec::with_capacity(20);
            for v in [20, 160, 4, 4, 8] {
                // cbSize, iWidth, iHorzGap, iVertGap, iArrange (ARW_HIDE)
                put(&mut o, v);
            }
            Outcome::Bytes(o)
        }
        SPI_GETANIMATION => {
            if cb_size != 8 {
                return Outcome::Error(ERROR_INVALID_PARAMETER);
            }
            let mut o = Vec::with_capacity(8);
            put(&mut o, 8);
            put(&mut o, 1); // iMinAnimate
            Outcome::Bytes(o)
        }
        SPI_GETFILTERKEYS => zeroed(cb_size, 24, None),
        SPI_GETTOGGLEKEYS => zeroed(cb_size, 8, None),
        SPI_GETSTICKYKEYS => zeroed(cb_size, 8, Some((4, 0x0000_0002))), // dwFlags: SKF_AVAILABLE
        SPI_GETMOUSEKEYS => zeroed(cb_size, 28, None),
        SPI_GETACCESSTIMEOUT => zeroed(cb_size, 12, None),
        SPI_GETHIGHCONTRAST => zeroed(cb_size, 16, None),
        _ => Outcome::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(o: Outcome) -> Vec<u8> {
        match o {
            Outcome::Bytes(b) => b,
            other => panic!("{other:?}"),
        }
    }

    fn i32_at(b: &[u8], at: usize) -> i32 {
        i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn plain_values_are_four_bytes() {
        assert_eq!(bytes(query(SPI_GETWHEELSCROLLLINES, 0, 0, [0; 4])), 3u32.to_le_bytes());
        assert_eq!(bytes(query(SPI_GETCARETWIDTH, 0, 0, [0; 4])), 1u32.to_le_bytes());
        assert_eq!(bytes(query(SPI_GETSCREENREADER, 0, 0, [0; 4])), 0u32.to_le_bytes());
        assert_eq!(bytes(query(SPI_GETMOUSEHOVERTIME, 0, 0, [0; 4])), 400u32.to_le_bytes());
    }

    #[test]
    fn work_area_is_a_rect() {
        let b = bytes(query(SPI_GETWORKAREA, 0, 0, [0, 0, 1024, 728]));
        assert_eq!((b.len(), i32_at(&b, 8), i32_at(&b, 12)), (16, 1024, 728));
    }

    #[test]
    fn non_client_metrics_check_cb_size() {
        let b = bytes(query(SPI_GETNONCLIENTMETRICS, 0, 504, [0; 4]));
        assert_eq!(b.len(), 504);
        assert_eq!(i32_at(&b, 0), 504);
        assert_eq!(i32_at(&b, 8), 17); // iScrollWidth
        assert_eq!(i32_at(&b, 24), -12); // lfCaptionFont.lfHeight
        assert_eq!(i32_at(&b, 500), 4); // iPaddedBorderWidth
        assert_eq!(bytes(query(SPI_GETNONCLIENTMETRICS, 0, 500, [0; 4])).len(), 500);
        assert_eq!(query(SPI_GETNONCLIENTMETRICS, 0, 100, [0; 4]), Outcome::Error(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn the_ui_font_is_segoe_ui() {
        let f = ui_font();
        let face: Vec<u16> =
            f[28..92].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        assert_eq!(alloc::string::String::from_utf16(&face).unwrap(), "Segoe UI");
        assert_eq!(bytes(query(SPI_GETICONTITLELOGFONT, 92, 0, [0; 4])).len(), 92);
        assert_eq!(query(SPI_GETICONTITLELOGFONT, 0, 0, [0; 4]), Outcome::Error(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn structures_start_with_their_size() {
        for (a, size) in
            [(SPI_GETFILTERKEYS, 24), (SPI_GETTOGGLEKEYS, 8), (SPI_GETHIGHCONTRAST, 16), (SPI_GETACCESSTIMEOUT, 12)]
        {
            assert!(has_cb_size(a));
            let b = bytes(query(a, 0, size, [0; 4]));
            assert_eq!((b.len() as u32, i32_at(&b, 0) as u32), (size, size));
            assert_eq!(query(a, 0, size + 1, [0; 4]), Outcome::Error(ERROR_INVALID_PARAMETER));
        }
        assert!(!has_cb_size(SPI_GETWORKAREA));
    }

    #[test]
    fn unknown_actions_are_left_to_the_caller() {
        assert_eq!(query(0x7777, 0, 0, [0; 4]), Outcome::Unknown);
        assert_eq!(query(0x0002 /* SPI_SETBEEP */, 0, 0, [0; 4]), Outcome::Unknown);
    }
}
