//! Dialogs without the machine (behaviour from the spec sheets of
//! `specs/windows/user32.dll/`): the dialog templates of
//! resource files, the numbers `GetDlgItemInt` and `SetDlgItemInt` read and
//! write, the order of the controls for the Tab key and the arrow keys, the
//! mnemonics of control texts, and the accelerator tables.
//!
//! Formats: Microsoft Learn, "DLGTEMPLATE", "DLGITEMTEMPLATE",
//! "DLGTEMPLATEEX", "DLGITEMTEMPLATEEX", "Dialog Box Styles", "ACCEL" and
//! "About Keyboard Accelerators"; constants from `winuser.h`.

use alloc::vec::Vec;

// ---- styles and ids (winuser.h)

pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_DISABLED: u32 = 0x0800_0000;
pub const WS_GROUP: u32 = 0x0002_0000;
pub const WS_TABSTOP: u32 = 0x0001_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_BORDER: u32 = 0x0080_0000;
pub const WS_DLGFRAME: u32 = 0x0040_0000;
pub const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
pub const WS_EX_DLGMODALFRAME: u32 = 0x0000_0001;
pub const WS_EX_CONTROLPARENT: u32 = 0x0001_0000;

pub const DS_ABSALIGN: u32 = 0x01;
pub const DS_SYSMODAL: u32 = 0x02;
pub const DS_3DLOOK: u32 = 0x04;
pub const DS_FIXEDSYS: u32 = 0x08;
pub const DS_NOFAILCREATE: u32 = 0x10;
pub const DS_LOCALEDIT: u32 = 0x20;
pub const DS_SETFONT: u32 = 0x40;
pub const DS_MODALFRAME: u32 = 0x80;
pub const DS_NOIDLEMSG: u32 = 0x100;
pub const DS_SETFOREGROUND: u32 = 0x200;
pub const DS_CONTROL: u32 = 0x400;
pub const DS_CENTER: u32 = 0x800;
pub const DS_CENTERMOUSE: u32 = 0x1000;
pub const DS_CONTEXTHELP: u32 = 0x2000;
pub const DS_SHELLFONT: u32 = DS_SETFONT | DS_FIXEDSYS;

pub const IDOK: u32 = 1;
pub const IDCANCEL: u32 = 2;

/// `DM_GETDEFID` / `DM_SETDEFID` and the `DC_HASDEFID` marker of the answer.
pub const DM_GETDEFID: u32 = 0x0400;
pub const DM_SETDEFID: u32 = 0x0401;
pub const DC_HASDEFID: u32 = 0x534B;

// WM_GETDLGCODE answers.
pub const DLGC_WANTARROWS: u32 = 0x0001;
pub const DLGC_WANTTAB: u32 = 0x0002;
pub const DLGC_WANTALLKEYS: u32 = 0x0004;
pub const DLGC_WANTMESSAGE: u32 = 0x0004;
pub const DLGC_HASSETSEL: u32 = 0x0008;
pub const DLGC_DEFPUSHBUTTON: u32 = 0x0010;
pub const DLGC_UNDEFPUSHBUTTON: u32 = 0x0020;
pub const DLGC_RADIOBUTTON: u32 = 0x0040;
pub const DLGC_WANTCHARS: u32 = 0x0080;
pub const DLGC_STATIC: u32 = 0x0100;
pub const DLGC_BUTTON: u32 = 0x2000;

/// The window procedures of the system classes are not addresses: the class
/// table keeps these small numbers (`user32.dll` runs them in the process of
/// the window, in `GetWindowLongPtr(GWLP_WNDPROC)` and `CallWindowProc` they
/// are what an application sees). 0 is `DefWindowProc`.
pub mod sysproc {
    pub const BUTTON: u64 = 1;
    pub const STATIC: u64 = 2;
    pub const EDIT: u64 = 3;
    pub const LISTBOX: u64 = 4;
    pub const COMBOBOX: u64 = 5;
    pub const SCROLLBAR: u64 = 6;
    pub const DIALOG: u64 = 7;
    pub const COMBOLBOX: u64 = 8;
    /// The popup menu class `#32768`.
    pub const MENU: u64 = 9;
    /// The largest value that is a system procedure.
    pub const MAX: u64 = 0xFF;
}

/// The classes a template names by number (`DLGITEMTEMPLATE`: 0xFFFF and
/// the ordinal).
pub fn predefined_class(ordinal: u16) -> Option<&'static str> {
    Some(match ordinal {
        0x0080 => "Button",
        0x0081 => "Edit",
        0x0082 => "Static",
        0x0083 => "ListBox",
        0x0084 => "ScrollBar",
        0x0085 => "ComboBox",
        _ => return None,
    })
}

/// A string or an ordinal (`sz_Or_Ord`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SzOrd {
    /// Nothing (a first word of 0).
    None,
    Ord(u16),
    Str(Vec<u16>),
}

impl SzOrd {
    /// The class name this stands for as a window class.
    pub fn class_name(&self) -> Option<Vec<u16>> {
        match self {
            SzOrd::None => None,
            SzOrd::Ord(o) => predefined_class(*o).map(|s| s.encode_utf16().collect()),
            SzOrd::Str(s) => Some(s.clone()),
        }
    }
}

/// The font a template asks for (`DS_SETFONT`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DlgFont {
    pub points: u16,
    pub weight: u16,
    pub italic: bool,
    pub charset: u8,
    pub face: Vec<u16>,
}

/// One control of a template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DlgItem {
    pub style: u32,
    pub ex_style: u32,
    pub help_id: u32,
    /// Dialog units.
    pub x: i16,
    pub y: i16,
    pub cx: i16,
    pub cy: i16,
    pub id: u32,
    pub class: SzOrd,
    pub title: SzOrd,
    /// The creation data (`lpParam` of the control's `CREATESTRUCT`).
    pub data: Vec<u8>,
}

/// A parsed dialog template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DlgTemplate {
    /// `DLGTEMPLATEEX`.
    pub ex: bool,
    pub style: u32,
    pub ex_style: u32,
    pub help_id: u32,
    pub x: i16,
    pub y: i16,
    pub cx: i16,
    pub cy: i16,
    pub menu: SzOrd,
    pub class: SzOrd,
    pub title: Vec<u16>,
    pub font: Option<DlgFont>,
    pub items: Vec<DlgItem>,
}

struct Rd<'a> {
    d: &'a [u8],
    at: usize,
}

impl Rd<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.d.get(self.at)?;
        self.at += 1;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        let b = self.d.get(self.at..self.at + 2)?;
        self.at += 2;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.d.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn align4(&mut self) {
        self.at = (self.at + 3) & !3;
    }
    fn align2(&mut self) {
        self.at = (self.at + 1) & !1;
    }
    /// A NUL-terminated UTF-16 string.
    fn sz(&mut self) -> Option<Vec<u16>> {
        let mut v = Vec::new();
        loop {
            let c = self.u16()?;
            if c == 0 {
                return Some(v);
            }
            v.push(c);
        }
    }
    /// `sz_Or_Ord`: 0, 0xFFFF and an ordinal, or a string.
    fn sz_or_ord(&mut self) -> Option<SzOrd> {
        let first = self.u16()?;
        match first {
            0 => Some(SzOrd::None),
            0xFFFF => Some(SzOrd::Ord(self.u16()?)),
            c => {
                let mut v = alloc::vec![c];
                v.extend(self.sz()?);
                Some(SzOrd::Str(v))
            }
        }
    }
}

/// Parse the data of an `RT_DIALOG` resource (or a template in memory).
/// `None` when it is cut short.
pub fn parse_template(data: &[u8]) -> Option<DlgTemplate> {
    let mut r = Rd { d: data, at: 0 };
    // The signature 0xFFFF marks an extended template whatever its `dlgVer` says (a version 2 header is
    // not rejected, DialogBoxIndirectParamW.yaml `extended_version`).
    let ex = data.len() >= 4 && data[2] == 0xFF && data[3] == 0xFF;
    let (style, ex_style, help_id, count);
    if ex {
        r.at = 4;
        help_id = r.u32()?;
        ex_style = r.u32()?;
        style = r.u32()?;
        count = r.u16()?;
    } else {
        help_id = 0;
        style = r.u32()?;
        ex_style = r.u32()?;
        count = r.u16()?;
    }
    let x = r.u16()? as i16;
    let y = r.u16()? as i16;
    let cx = r.u16()? as i16;
    let cy = r.u16()? as i16;
    let menu = r.sz_or_ord()?;
    let class = r.sz_or_ord()?;
    let title = r.sz()?;
    let font = if style & DS_SETFONT != 0 {
        let points = r.u16()?;
        let (weight, italic, charset) = if ex { (r.u16()?, r.u8()? != 0, r.u8()?) } else { (400, false, 1) };
        Some(DlgFont { points, weight, italic, charset, face: r.sz()? })
    } else {
        None
    };
    let mut items = Vec::new();
    for _ in 0..count {
        r.align4();
        let (style, ex_style, help_id);
        if ex {
            help_id = r.u32()?;
            ex_style = r.u32()?;
            style = r.u32()?;
        } else {
            style = r.u32()?;
            ex_style = r.u32()?;
            help_id = 0;
        }
        let x = r.u16()? as i16;
        let y = r.u16()? as i16;
        let cx = r.u16()? as i16;
        let cy = r.u16()? as i16;
        let id = if ex { r.u32()? } else { u32::from(r.u16()?) };
        let class = r.sz_or_ord()?;
        let title = r.sz_or_ord()?;
        let n = usize::from(r.u16()?);
        // The creation data: in the old format `n` bytes (the count includes
        // its own two bytes in `DLGITEMTEMPLATEEX`: Learn says "the length,
        // in bytes, of the creation data ... if zero there is none").
        let data_len = if n == 0 {
            0
        } else if ex {
            n.saturating_sub(2)
        } else {
            n
        };
        let bytes = r.d.get(r.at..r.at + data_len)?.to_vec();
        r.at += data_len;
        items.push(DlgItem { style, ex_style, help_id, x, y, cx, cy, id, class, title, data: bytes });
    }
    r.align2();
    Some(DlgTemplate { ex, style, ex_style, help_id, x, y, cx, cy, menu, class, title, font, items })
}

/// The style and extended style the dialog window gets: the template's, with
/// the bits every dialog has on the oracle (`WS_CLIPSIBLINGS`, `DS_3DLOOK`,
/// `WS_EX_DLGMODALFRAME` for `DS_MODALFRAME`, `WS_EX_CONTROLPARENT`;
/// `DialogBoxParamW.tests/dbp_init.rs`: 0x80c800c0 becomes 0x84c800c4 and
/// extended 0x10101 with the computed `WS_EX_WINDOWEDGE`).
pub fn window_styles(t: &DlgTemplate) -> (u32, u32) {
    let mut style = t.style;
    let mut ex = t.ex_style;
    style |= WS_CLIPSIBLINGS | DS_3DLOOK;
    if style & DS_MODALFRAME != 0 {
        ex |= WS_EX_DLGMODALFRAME;
    }
    ex |= WS_EX_CONTROLPARENT;
    (style, ex)
}

// ---- dialog units

/// Dialog base units: the average character width and the height of the
/// dialog font (Learn, `GetDialogBaseUnits`/`MapDialogRect`: the width of
/// the 52 letters divided by 52, rounded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseUnits {
    pub x: i32,
    pub y: i32,
}

impl BaseUnits {
    /// From the width of "abc...xyzABC...XYZ" and the font's height.
    pub fn from_extent(width_of_52_letters: i32, height: i32) -> BaseUnits {
        BaseUnits { x: (width_of_52_letters / 26 + 1) / 2, y: height }
    }
    pub fn x_px(&self, du: i32) -> i32 {
        mul_div(du, self.x, 4)
    }
    pub fn y_px(&self, du: i32) -> i32 {
        mul_div(du, self.y, 8)
    }
}

/// `MulDiv` as `MapDialogRect` rounds (to nearest, halves away from zero).
pub fn mul_div(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return -1;
    }
    let p = i64::from(a) * i64::from(b);
    let c = i64::from(c);
    let half = c.abs() / 2;
    let r = if (p < 0) == (c < 0) { (p + half) / c } else { (p - half) / c };
    r as i32
}

// ---- numbers

/// What `GetDlgItemInt` makes of a control's text (`GetDlgItemInt.yaml`):
/// the value and whether the whole text was a number. Leading spaces (not
/// tabs) are skipped; with `signed` every leading `-` toggles the sign;
/// digits (ASCII and full-width) are read to the first other character
/// (not translated when something follows); a magnitude over 32 bits gives
/// 0, not translated.
pub fn parse_int(text: &[u16], signed: bool) -> (u32, bool) {
    let mut i = 0;
    while i < text.len() && text[i] == 0x20 {
        i += 1;
    }
    let mut neg = false;
    if signed {
        while i < text.len() && text[i] == u16::from(b'-') {
            neg = !neg;
            i += 1;
        }
    }
    let digit = |c: u16| -> Option<u32> {
        match c {
            0x30..=0x39 => Some(u32::from(c - 0x30)),
            0xFF10..=0xFF19 => Some(u32::from(c - 0xFF10)),
            _ => None,
        }
    };
    let start = i;
    let mut m: u64 = 0;
    let mut over = false;
    while i < text.len() {
        let Some(d) = digit(text[i]) else { break };
        m = m * 10 + u64::from(d);
        if m > 0xFFFF_FFFF {
            over = true;
            m = 0xFFFF_FFFF + 1;
        }
        i += 1;
    }
    if i == start || over {
        return (0, false);
    }
    let whole = i == text.len();
    let m = m as u32;
    if signed {
        if neg {
            if m > 0x8000_0000 {
                return (0, false);
            }
            if m == 0x8000_0000 {
                return (0x8000_0000, false);
            }
            return ((m as i32).wrapping_neg() as u32, whole);
        }
        if m > 0x7FFF_FFFF {
            return (0, false);
        }
        return (m, whole);
    }
    (m, whole)
}

/// The text `SetDlgItemInt` writes: unsigned digits, or with `signed` a
/// minus sign and the magnitude. i32::MIN gives "-0" on the oracle
/// (`GetDlgItemInt.tests/sdi_int.rs`).
pub fn format_int(value: u32, signed: bool) -> Vec<u16> {
    let mut out = Vec::new();
    let mut v = value;
    if signed && (value as i32) < 0 {
        out.push(u16::from(b'-'));
        v = (value as i32).wrapping_neg() as u32;
        if (v as i32) < 0 {
            v = 0;
        }
    }
    let mut digits = Vec::new();
    if v == 0 {
        digits.push(b'0' as u16);
    }
    while v > 0 {
        digits.push(b'0' as u16 + (v % 10) as u16);
        v /= 10;
    }
    digits.reverse();
    out.extend(digits);
    out
}

// ---- the order of controls

/// What the navigation needs of a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavCtl {
    pub id: u32,
    pub style: u32,
    pub visible: bool,
    pub enabled: bool,
}

impl NavCtl {
    pub fn usable(&self) -> bool {
        self.visible && self.enabled
    }
    pub fn group(&self) -> bool {
        self.style & WS_GROUP != 0
    }
    pub fn tabstop(&self) -> bool {
        self.style & WS_TABSTOP != 0
    }
}

/// `GetNextDlgGroupItem`: `ctls` are the dialog's children in z order and
/// `from` the index of the control to start at (`None`: the first one). The
/// group of `from` runs from the last control with `WS_GROUP` at or before it
/// up to (not including) the next one; the walk wraps inside the group and
/// skips what is hidden or disabled; with nothing else usable the result is
/// `from` itself (`GetNextDlgTabItem.yaml` `group_items`, `no_usable`).
pub fn next_group_item(ctls: &[NavCtl], from: Option<usize>, previous: bool) -> Option<usize> {
    if ctls.is_empty() {
        return None;
    }
    let Some(cur) = from else {
        // NULL starts at the first control of the dialog.
        return Some(if previous { ctls.len() - 1 } else { 0 });
    };
    let mut start = cur;
    while start > 0 && !ctls[start].group() {
        start -= 1;
    }
    let mut end = cur + 1;
    while end < ctls.len() && !ctls[end].group() {
        end += 1;
    }
    let n = end - start;
    let mut at = cur;
    for _ in 0..n {
        at = if previous {
            if at == start { end - 1 } else { at - 1 }
        } else if at + 1 == end {
            start
        } else {
            at + 1
        };
        if at == cur {
            break;
        }
        if ctls[at].usable() {
            return Some(at);
        }
    }
    Some(cur)
}

/// `GetNextDlgTabItem`: the next (or `previous`) control with `WS_TABSTOP`
/// that is visible and enabled, starting beside `from` (`None`: forward gives
/// the first tab stop, backward `None`: nothing before the start) and
/// wrapping when a control was given.
pub fn next_tab_item(ctls: &[NavCtl], from: Option<usize>, previous: bool) -> Option<usize> {
    let n = ctls.len();
    if n == 0 {
        return None;
    }
    let ok = |i: usize| ctls[i].tabstop() && ctls[i].usable();
    match from {
        None => {
            if previous {
                None
            } else {
                (0..n).find(|&i| ok(i))
            }
        }
        Some(cur) => {
            for step in 1..=n {
                let i = if previous { (cur + n - step % n) % n } else { (cur + step) % n };
                if ok(i) {
                    return Some(i);
                }
            }
            None
        }
    }
}

// ---- mnemonics

/// The mnemonic of a control text: the character after the first `&` that
/// is not itself `&&` (lower-cased for ASCII letters), if any.
pub fn mnemonic(text: &[u16]) -> Option<u16> {
    let mut i = 0;
    while i < text.len() {
        if text[i] == u16::from(b'&') {
            match text.get(i + 1) {
                Some(&n) if n == u16::from(b'&') => i += 2,
                Some(&n) => return Some(fold(n)),
                None => return None,
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Lower case for the letters a mnemonic can be (ASCII and Latin-1).
pub fn fold(c: u16) -> u16 {
    match c {
        0x41..=0x5A => c + 0x20,
        0xC0..=0xDE if c != 0xD7 => c + 0x20,
        _ => c,
    }
}

// ---- accelerators

pub const FVIRTKEY: u8 = 0x01;
pub const FNOINVERT: u8 = 0x02;
pub const FSHIFT: u8 = 0x04;
pub const FCONTROL: u8 = 0x08;
pub const FALT: u8 = 0x10;
/// The marker of the last entry in a resource.
pub const ACCEL_LAST: u8 = 0x80;

/// An accelerator (`ACCEL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accel {
    pub flags: u8,
    pub key: u16,
    pub cmd: u16,
}

/// The entries of an `RT_ACCELERATOR` resource (`ACCELTABLEENTRY`: 8 bytes,
/// flags word, key, id, padding; the last entry has `0x80`).
pub fn parse_accelerators(data: &[u8]) -> Vec<Accel> {
    let mut out = Vec::new();
    for c in data.chunks_exact(8) {
        let flags = c[0];
        out.push(Accel {
            flags: flags & !ACCEL_LAST,
            key: u16::from_le_bytes([c[2], c[3]]),
            cmd: u16::from_le_bytes([c[4], c[5]]),
        });
        if flags & ACCEL_LAST != 0 {
            break;
        }
    }
    out
}

/// The modifier keys down while a key message is translated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// What `TranslateAcceleratorW` finds for a key message `msg` (`WM_KEYDOWN`
/// 0x100, `WM_SYSKEYDOWN` 0x104, `WM_CHAR` 0x102, `WM_SYSCHAR` 0x106; others
/// never match) with key `key`: the command (`TranslateAcceleratorW.yaml`,
/// `LoadAcceleratorsW.tests/ac_table.rs`):
/// - a virtual-key entry matches `WM_KEYDOWN` and `WM_SYSKEYDOWN` of that
///   virtual key when Shift, Ctrl and Alt are as the entry says;
/// - a character entry matches `WM_CHAR` (and `WM_SYSCHAR` with Alt) of that
///   character, case sensitive (`WM_KEYDOWN` of the letter does not).
pub fn match_accel(table: &[Accel], msg: u32, key: u16, mods: Mods) -> Option<Accel> {
    let down = msg == 0x0100 || msg == 0x0104;
    let chr = msg == 0x0102 || msg == 0x0106;
    if !down && !chr {
        return None;
    }
    for a in table {
        if a.flags & FVIRTKEY != 0 {
            if !down || a.key != key {
                continue;
            }
            let want = Mods { shift: a.flags & FSHIFT != 0, ctrl: a.flags & FCONTROL != 0, alt: a.flags & FALT != 0 };
            // The modifiers are the keys' state (a `WM_SYSKEYDOWN` made by hand
            // without Alt down does not match an Alt entry:
            // `LoadAcceleratorsW.tests/ac_table.rs`).
            if want == mods {
                return Some(*a);
            }
        } else if chr && a.key == key {
            return Some(*a);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn ints() {
        assert_eq!(parse_int(&w("123"), false), (123, true));
        assert_eq!(parse_int(&w("  123"), false), (123, true));
        assert_eq!(parse_int(&w("\t123"), false), (0, false));
        assert_eq!(parse_int(&w("-123"), true), ((-123i32) as u32, true));
        assert_eq!(parse_int(&w("-123"), false), (0, false));
        assert_eq!(parse_int(&w("--42"), true), (42, true));
        assert_eq!(parse_int(&w("+123"), true), (0, false));
        assert_eq!(parse_int(&w("123  "), false), (123, false));
        assert_eq!(parse_int(&w("12 3"), false), (12, false));
        assert_eq!(parse_int(&w("1e3"), false), (1, false));
        assert_eq!(parse_int(&w("0x10"), false), (0, false));
        assert_eq!(parse_int(&w("007"), false), (7, true));
        assert_eq!(parse_int(&w("4294967295"), false), (0xFFFF_FFFF, true));
        assert_eq!(parse_int(&w("4294967296"), false), (0, false));
        assert_eq!(parse_int(&w("4294967295"), true), (0, false));
        assert_eq!(parse_int(&w("2147483647"), true), (0x7FFF_FFFF, true));
        assert_eq!(parse_int(&w("2147483648"), true), (0, false));
        assert_eq!(parse_int(&w("-2147483648"), true), (0x8000_0000, false));
        assert_eq!(parse_int(&w("-2147483649"), true), (0, false));
        assert_eq!(parse_int(&w("-"), true), (0, false));
        assert_eq!(parse_int(&w(""), false), (0, false));
        assert_eq!(parse_int(&[0xFF11, 0xFF12], false), (12, true));
        assert_eq!(parse_int(&w("-0"), true), (0, true));
        assert_eq!(parse_int(&w("-0"), false), (0, false));
    }

    #[test]
    fn int_text() {
        assert_eq!(format_int(123, false), w("123"));
        assert_eq!(format_int((-5i32) as u32, true), w("-5"));
        assert_eq!(format_int((-5i32) as u32, false), w("4294967291"));
        assert_eq!(format_int(0xFFFF_FFFF, true), w("-1"));
        assert_eq!(format_int(0, true), w("0"));
        assert_eq!(format_int(i32::MIN as u32, true), w("-0"));
    }

    #[test]
    fn units() {
        let b = BaseUnits { x: 6, y: 13 };
        assert_eq!(b.x_px(10), 15);
        assert_eq!(b.y_px(10), 16);
        assert_eq!(BaseUnits::from_extent(52 * 6, 13), BaseUnits { x: 6, y: 13 });
    }

    fn radios() -> Vec<NavCtl> {
        // 1003 check, 1004..1006 radios (1004 group+tabstop), 1007 tri-state (group+tabstop)
        let c = |id, style| NavCtl { id, style, visible: true, enabled: true };
        vec![
            c(1001, 0),
            c(1002, WS_TABSTOP),
            c(1003, WS_TABSTOP),
            c(1004, WS_GROUP | WS_TABSTOP),
            c(1005, 0),
            c(1006, 0),
            c(1007, WS_GROUP | WS_TABSTOP),
        ]
    }

    #[test]
    fn groups() {
        let mut c = radios();
        assert_eq!(next_group_item(&c, Some(4), false), Some(5));
        assert_eq!(next_group_item(&c, Some(5), false), Some(3));
        assert_eq!(next_group_item(&c, Some(3), true), Some(5));
        c[5].enabled = false;
        assert_eq!(next_group_item(&c, Some(4), false), Some(3));
        assert_eq!(next_group_item(&c, Some(3), true), Some(4));
        c[4].visible = false;
        assert_eq!(next_group_item(&c, Some(3), false), Some(3));
        assert_eq!(next_group_item(&c, None, false), Some(0));
    }

    #[test]
    fn tabs() {
        let c = radios();
        assert_eq!(next_tab_item(&c, Some(3), false), Some(6));
        assert_eq!(next_tab_item(&c, Some(4), false), Some(6));
        assert_eq!(next_tab_item(&c, Some(4), true), Some(3));
        assert_eq!(next_tab_item(&c, Some(1), true), Some(6));
        assert_eq!(next_tab_item(&c, None, false), Some(1));
        assert_eq!(next_tab_item(&c, None, true), None);
        assert_eq!(next_tab_item(&c, Some(6), false), Some(1));
    }

    #[test]
    fn mnemonics() {
        assert_eq!(mnemonic(&w("&Name:")), Some(b'n' as u16));
        assert_eq!(mnemonic(&w("T&wo")), Some(b'w' as u16));
        assert_eq!(mnemonic(&w("A&&B")), None);
        assert_eq!(mnemonic(&w("A&&B&c")), Some(b'c' as u16));
        assert_eq!(mnemonic(&w("OK")), None);
        assert_eq!(mnemonic(&w("end&")), None);
    }

    #[test]
    fn accelerators() {
        let table = vec![
            Accel { flags: FVIRTKEY, key: 0x70, cmd: 303 },
            Accel { flags: FVIRTKEY | FCONTROL | FALT, key: 0x4F, cmd: 301 },
            Accel { flags: 0, key: b'A' as u16, cmd: 304 },
            Accel { flags: FVIRTKEY | FSHIFT, key: 0x74, cmd: 305 },
            Accel { flags: FVIRTKEY | FALT, key: 0x0D, cmd: 306 },
        ];
        let none = Mods::default();
        assert_eq!(match_accel(&table, 0x100, 0x70, none).map(|a| a.cmd), Some(303));
        assert_eq!(match_accel(&table, 0x101, 0x70, none), None);
        assert_eq!(match_accel(&table, 0x102, b'A' as u16, none).map(|a| a.cmd), Some(304));
        assert_eq!(match_accel(&table, 0x102, b'a' as u16, none), None);
        assert_eq!(match_accel(&table, 0x100, b'A' as u16, none), None);
        assert_eq!(match_accel(&table, 0x100, 0x4F, none), None);
        assert_eq!(match_accel(&table, 0x100, 0x74, none), None);
        assert_eq!(match_accel(&table, 0x104, 0x0D, none), None);
        assert_eq!(match_accel(&table, 0x104, 0x0D, Mods { alt: true, ..none }).map(|a| a.cmd), Some(306));
        assert_eq!(match_accel(&table, 0x104, 0x70, none).map(|a| a.cmd), Some(303));
        assert_eq!(match_accel(&table, 0x200, 0, none), None);
    }

    #[test]
    fn parse_old_template() {
        // style, ex, 1 item, 5 6 7 8, menu 0, class 0, title "T"
        let mut d = Vec::new();
        d.extend(0x8000_00C0u32.to_le_bytes());
        d.extend(0u32.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        for v in [5i16, 6, 7, 8] {
            d.extend(v.to_le_bytes());
        }
        d.extend([0, 0, 0, 0]);
        d.extend([b'T', 0, 0, 0]);
        // font 8 "F"
        d.extend(8u16.to_le_bytes());
        d.extend([b'F', 0, 0, 0]);
        while d.len() % 4 != 0 {
            d.push(0);
        }
        // item: style, ex, x y cx cy, id 9, class 0xFFFF 0x80, title "OK", 0 extra
        d.extend(0x5001_0001u32.to_le_bytes());
        d.extend(0u32.to_le_bytes());
        for v in [1i16, 2, 3, 4] {
            d.extend(v.to_le_bytes());
        }
        d.extend(9u16.to_le_bytes());
        d.extend([0xFF, 0xFF, 0x80, 0]);
        d.extend([b'O', 0, b'K', 0, 0, 0]);
        d.extend(0u16.to_le_bytes());
        let t = parse_template(&d).unwrap();
        assert!(!t.ex);
        assert_eq!((t.x, t.y, t.cx, t.cy), (5, 6, 7, 8));
        assert_eq!(t.title, w("T"));
        assert_eq!(t.font.as_ref().unwrap().face, w("F"));
        assert_eq!(t.items.len(), 1);
        assert_eq!(t.items[0].id, 9);
        assert_eq!(t.items[0].class.class_name(), Some(w("Button")));
        assert_eq!(t.items[0].title, SzOrd::Str(w("OK")));
        let (s, ex) = window_styles(&t);
        assert_eq!(s, 0x8400_00C4);
        assert_eq!(ex, 0x0001_0001);
        assert_eq!(parse_template(&d[..10]), None);
    }
}
