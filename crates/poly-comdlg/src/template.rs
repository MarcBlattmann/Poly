//! In-memory dialog templates: a `DLGTEMPLATE` followed by its
//! `DLGITEMTEMPLATE`s (Learn, "Dialog Box Templates in Memory"; `winuser.h`
//! for the styles). The common dialogs are built from these and run by
//! `DialogBoxIndirectParamW`, so no resource file is needed.

use alloc::vec::Vec;

/// Window and dialog styles (`winuser.h`).
pub mod style {
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const WS_CHILD: u32 = 0x4000_0000;
    pub const WS_VISIBLE: u32 = 0x1000_0000;
    pub const WS_DISABLED: u32 = 0x0800_0000;
    pub const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
    pub const WS_CAPTION: u32 = 0x00C0_0000;
    pub const WS_BORDER: u32 = 0x0080_0000;
    pub const WS_DLGFRAME: u32 = 0x0040_0000;
    pub const WS_VSCROLL: u32 = 0x0020_0000;
    pub const WS_SYSMENU: u32 = 0x0008_0000;
    pub const WS_THICKFRAME: u32 = 0x0004_0000;
    pub const WS_GROUP: u32 = 0x0002_0000;
    pub const WS_TABSTOP: u32 = 0x0001_0000;
    pub const DS_MODALFRAME: u32 = 0x80;
    pub const DS_SETFONT: u32 = 0x40;
    pub const DS_3DLOOK: u32 = 0x4;
    pub const DS_CENTER: u32 = 0x800;
    pub const DS_CONTROL: u32 = 0x400;
    pub const WS_EX_CONTROLPARENT: u32 = 0x10000;
    pub const BS_PUSHBUTTON: u32 = 0;
    pub const BS_DEFPUSHBUTTON: u32 = 1;
    pub const BS_AUTOCHECKBOX: u32 = 3;
    pub const BS_GROUPBOX: u32 = 7;
    pub const ES_AUTOHSCROLL: u32 = 0x80;
    pub const ES_NUMBER: u32 = 0x2000;
    pub const ES_READONLY: u32 = 0x800;
    pub const SS_LEFT: u32 = 0;
    pub const SS_NOPREFIX: u32 = 0x80;
    pub const SS_ETCHEDFRAME: u32 = 0x12;
    pub const LBS_NOTIFY: u32 = 1;
    pub const LBS_NOINTEGRALHEIGHT: u32 = 0x100;
}

/// The window class of a control: a predefined one by atom, or a registered name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class<'a> {
    Button,
    Edit,
    Static,
    ListBox,
    ScrollBar,
    ComboBox,
    Name(&'a str),
}

impl Class<'_> {
    /// The atom `DLGITEMTEMPLATE` uses for a predefined class.
    fn atom(&self) -> Option<u16> {
        Some(match self {
            Class::Button => 0x0080,
            Class::Edit => 0x0081,
            Class::Static => 0x0082,
            Class::ListBox => 0x0083,
            Class::ScrollBar => 0x0084,
            Class::ComboBox => 0x0085,
            Class::Name(_) => return None,
        })
    }
}

/// One control. Positions and sizes are dialog units.
#[derive(Clone, Debug)]
pub struct Item<'a> {
    pub class: Class<'a>,
    pub text: &'a str,
    pub id: u16,
    pub style: u32,
    pub ex_style: u32,
    pub x: i16,
    pub y: i16,
    pub cx: i16,
    pub cy: i16,
}

/// A dialog.
#[derive(Clone, Debug)]
pub struct Dialog<'a> {
    pub style: u32,
    pub ex_style: u32,
    pub x: i16,
    pub y: i16,
    pub cx: i16,
    pub cy: i16,
    pub title: &'a str,
    /// Point size and face (sets `DS_SETFONT`).
    pub font: Option<(u16, &'a str)>,
    pub items: Vec<Item<'a>>,
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put_sz(v: &mut Vec<u8>, s: &str) {
    for c in s.encode_utf16() {
        put16(v, c);
    }
    put16(v, 0);
}

fn align4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

impl Dialog<'_> {
    /// The template as bytes (the old `DLGTEMPLATE`, not the extended one).
    pub fn build(&self) -> Vec<u8> {
        let mut v = Vec::new();
        let mut st = self.style;
        if self.font.is_some() {
            st |= style::DS_SETFONT;
        }
        put32(&mut v, st);
        put32(&mut v, self.ex_style);
        put16(&mut v, self.items.len() as u16);
        for n in [self.x, self.y, self.cx, self.cy] {
            put16(&mut v, n as u16);
        }
        // No menu, the default dialog class.
        put16(&mut v, 0);
        put16(&mut v, 0);
        put_sz(&mut v, self.title);
        if let Some((pt, face)) = self.font {
            put16(&mut v, pt);
            put_sz(&mut v, face);
        }
        for it in &self.items {
            align4(&mut v);
            put32(&mut v, it.style);
            put32(&mut v, it.ex_style);
            for n in [it.x, it.y, it.cx, it.cy] {
                put16(&mut v, n as u16);
            }
            put16(&mut v, it.id);
            match it.class.atom() {
                Some(a) => {
                    put16(&mut v, 0xFFFF);
                    put16(&mut v, a);
                }
                None => {
                    if let Class::Name(n) = it.class {
                        put_sz(&mut v, n);
                    }
                }
            }
            put_sz(&mut v, it.text);
            // No creation data.
            put16(&mut v, 0);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;

    fn r16(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }
    fn r32(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }
    /// A zero-terminated string at `at`; the position after it.
    fn rsz(b: &[u8], mut at: usize) -> (String, usize) {
        let mut u = Vec::new();
        while r16(b, at) != 0 {
            u.push(r16(b, at));
            at += 2;
        }
        (String::from_utf16(&u).unwrap(), at + 2)
    }

    #[test]
    fn a_template_reads_back() {
        let d = Dialog {
            style: style::WS_POPUP | style::WS_CAPTION | style::DS_MODALFRAME,
            ex_style: 0,
            x: 1,
            y: 2,
            cx: 100,
            cy: 50,
            title: "Open",
            font: Some((8, "MS Shell Dlg")),
            items: vec![
                Item {
                    class: Class::Button,
                    text: "OK",
                    id: 1,
                    style: style::WS_CHILD | style::BS_DEFPUSHBUTTON,
                    ex_style: 0,
                    x: 5,
                    y: 30,
                    cx: 40,
                    cy: 12,
                },
                Item {
                    class: Class::Name("PolyColorGrid"),
                    text: "",
                    id: 720,
                    style: style::WS_CHILD,
                    ex_style: 0,
                    x: 5,
                    y: 5,
                    cx: 60,
                    cy: 40,
                },
                Item {
                    class: Class::ListBox,
                    text: "",
                    id: 1120,
                    style: style::WS_CHILD,
                    ex_style: 0,
                    x: 0,
                    y: 0,
                    cx: 1,
                    cy: 1,
                },
            ],
        };
        let b = d.build();
        assert_eq!(r32(&b, 0), d.style | style::DS_SETFONT);
        assert_eq!(r16(&b, 8), 3);
        assert_eq!((r16(&b, 10), r16(&b, 12), r16(&b, 14), r16(&b, 16)), (1, 2, 100, 50));
        assert_eq!((r16(&b, 18), r16(&b, 20)), (0, 0));
        let (title, at) = rsz(&b, 22);
        assert_eq!(title, "Open");
        assert_eq!(r16(&b, at), 8);
        let (face, at) = rsz(&b, at + 2);
        assert_eq!(face, "MS Shell Dlg");
        // The first item is DWORD aligned.
        let at = at.div_ceil(4) * 4;
        assert_eq!(r32(&b, at), style::WS_CHILD | style::BS_DEFPUSHBUTTON);
        assert_eq!((r16(&b, at + 8), r16(&b, at + 10), r16(&b, at + 12), r16(&b, at + 14)), (5, 30, 40, 12));
        assert_eq!(r16(&b, at + 16), 1);
        assert_eq!((r16(&b, at + 18), r16(&b, at + 20)), (0xFFFF, 0x0080));
        let (text, at) = rsz(&b, at + 22);
        assert_eq!(text, "OK");
        assert_eq!(r16(&b, at), 0);
        // The second has a class by name.
        let at = (at + 2).div_ceil(4) * 4;
        assert_eq!(r16(&b, at + 16), 720);
        let (class, _) = rsz(&b, at + 18);
        assert_eq!(class, "PolyColorGrid");
        assert_eq!(b.len() % 2, 0);
    }
}
