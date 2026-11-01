//! Window properties of the ICCCM (2.0, "Client Properties" and "Window
//! Manager Properties") and the EWMH (1.5, "Application Window
//! Properties"), as a window manager reads and writes them.
//!
//! | Property | Type, format | Layout |
//! |---|---|---|
//! | `WM_NAME` | `STRING` (Latin-1), `COMPOUND_TEXT` or `UTF8_STRING`, 8 | the text |
//! | `_NET_WM_NAME` | `UTF8_STRING`, 8 | the text |
//! | `WM_CLASS` | `STRING`, 8 | instance and class, each NUL-terminated |
//! | `WM_PROTOCOLS` | `ATOM`, 32 | the protocols the client takes part in |
//! | `WM_HINTS` | `WM_HINTS`, 32 | flags, input, initial state, icon pixmap, icon window, icon x, y, icon mask, window group |
//! | `WM_NORMAL_HINTS` | `WM_SIZE_HINTS`, 32 | flags, 4 unused, min w, h, max w, h, increments w, h, min aspect, max aspect, base w, h, gravity |
//! | `WM_TRANSIENT_FOR` | `WINDOW`, 32 | the window it is transient for |
//! | `_NET_WM_WINDOW_TYPE` | `ATOM`, 32 | types, most preferred first |
//! | `_MOTIF_WM_HINTS` | itself, 32 | flags, functions, decorations, input mode, status (Motif's `Xm/MwmUtil.h`) |
//! | `WM_STATE` (written) | `WM_STATE`, 32 | state (0 withdrawn, 1 normal, 3 iconic), icon window |

use crate::Xid;
use crate::request::PropertyReply;
use alloc::string::String;
use alloc::vec::Vec;

/// `WM_HINTS` flag: the input field is set.
pub const INPUT_HINT: u32 = 1;
/// `WM_NORMAL_HINTS` flags.
pub const P_MIN_SIZE: u32 = 16;
pub const P_MAX_SIZE: u32 = 32;
/// `_MOTIF_WM_HINTS` flag: the decorations field is set.
pub const MWM_HINTS_DECORATIONS: u32 = 2;

/// `WM_STATE` states.
pub mod state {
    pub const WITHDRAWN: u32 = 0;
    pub const NORMAL: u32 = 1;
    pub const ICONIC: u32 = 3;
}

/// Text of a `STRING` (ISO Latin-1), `UTF8_STRING` or `COMPOUND_TEXT`
/// property (of which only the Latin-1 part is understood: its escape
/// sequences are dropped).
pub fn text(p: &PropertyReply, utf8_string: Xid, compound_text: Xid) -> Option<String> {
    if p.format != 8 {
        return None;
    }
    let bytes = p.value.split(|&b| b == 0).next().unwrap_or_default();
    if p.ty == utf8_string {
        return Some(String::from_utf8_lossy(bytes).into_owned());
    }
    if p.ty == crate::atoms::predefined::STRING || p.ty == compound_text {
        let mut out = String::new();
        let mut it = bytes.iter().copied().peekable();
        while let Some(b) = it.next() {
            if b == 0x1b && p.ty == compound_text {
                // An escape sequence: intermediate bytes, then a final byte.
                while it.next_if(|c| (0x20..=0x2f).contains(c)).is_some() {}
                it.next();
                continue;
            }
            out.push(char::from(b));
        }
        return Some(out);
    }
    None
}

/// `WM_CLASS`: (instance, class).
pub fn class(p: &PropertyReply) -> (String, String) {
    let mut parts = p.value.split(|&b| b == 0).map(|s| String::from_utf8_lossy(s).into_owned());
    (parts.next().unwrap_or_default(), parts.next().unwrap_or_default())
}

/// `WM_HINTS`' input field: whether the client wants the window manager
/// to give it the keyboard (true when not set, as the ICCCM recommends
/// window managers assume).
pub fn input_hint(p: &PropertyReply) -> bool {
    let v = p.u32s();
    match v.first() {
        Some(flags) if flags & INPUT_HINT != 0 => v.get(1).is_none_or(|&i| i != 0),
        _ => true,
    }
}

/// The size limits of `WM_NORMAL_HINTS`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizeHints {
    pub min: Option<(u32, u32)>,
    pub max: Option<(u32, u32)>,
}

impl SizeHints {
    pub fn parse(p: &PropertyReply) -> SizeHints {
        let v = p.u32s();
        let at = |i: usize| v.get(i).copied().unwrap_or(0);
        let flags = at(0);
        let pair = |i: usize| (at(i), at(i + 1));
        SizeHints {
            min: (flags & P_MIN_SIZE != 0 && v.len() >= 7).then(|| pair(5)),
            max: (flags & P_MAX_SIZE != 0 && v.len() >= 9).then(|| pair(7)),
        }
    }

    /// `(w, h)` within the limits.
    pub fn constrain(&self, w: u32, h: u32) -> (u32, u32) {
        let (mut w, mut h) = (w, h);
        if let Some((mw, mh)) = self.max.filter(|(mw, mh)| *mw > 0 && *mh > 0) {
            (w, h) = (w.min(mw), h.min(mh));
        }
        if let Some((mw, mh)) = self.min {
            (w, h) = (w.max(mw), h.max(mh));
        }
        (w.max(1), h.max(1))
    }
}

/// `_MOTIF_WM_HINTS`: `Some(false)` if the client asks for no
/// decorations, `Some(true)` for some, `None` if it does not say.
pub fn motif_decorations(p: &PropertyReply) -> Option<bool> {
    let v = p.u32s();
    let flags = *v.first()?;
    (flags & MWM_HINTS_DECORATIONS != 0).then(|| v.get(2).is_some_and(|&d| d != 0))
}

/// A list of 32-bit values as property data.
pub fn u32_data(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atoms::predefined::{ATOM, STRING};
    use alloc::vec;

    const UTF8: Xid = 300;
    const CTEXT: Xid = 301;

    fn prop(ty: Xid, format: u8, value: &[u8]) -> PropertyReply {
        PropertyReply { format, ty, bytes_after: 0, value: value.to_vec() }
    }

    fn prop32(ty: Xid, values: &[u32]) -> PropertyReply {
        prop(ty, 32, &u32_data(values))
    }

    #[test]
    fn decodes_titles_of_each_type() {
        assert_eq!(text(&prop(UTF8, 8, "Grüße".as_bytes()), UTF8, CTEXT).as_deref(), Some("Grüße"));
        assert_eq!(text(&prop(STRING, 8, b"caf\xe9"), UTF8, CTEXT).as_deref(), Some("café"));
        let ct = b"a\x1b-Ab\x1b%Gc";
        assert_eq!(text(&prop(CTEXT, 8, ct), UTF8, CTEXT).as_deref(), Some("abc"));
        assert_eq!(text(&prop(STRING, 8, b"xeyes\0junk"), UTF8, CTEXT).as_deref(), Some("xeyes"));
        assert_eq!(text(&prop(ATOM, 32, &[0; 4]), UTF8, CTEXT), None);
        assert_eq!(text(&prop(0, 0, b""), UTF8, CTEXT), None, "no property");
    }

    #[test]
    fn reads_class_hints_and_motif_hints() {
        assert_eq!(class(&prop(STRING, 8, b"xterm\0XTerm\0")), ("xterm".into(), "XTerm".into()));
        assert_eq!(class(&prop(0, 0, b"")), (String::new(), String::new()));
        assert!(input_hint(&prop32(35, &[INPUT_HINT, 1, 1])));
        assert!(!input_hint(&prop32(35, &[INPUT_HINT, 0, 1])));
        assert!(input_hint(&prop32(35, &[0, 0])), "input not set: assume true");
        assert!(input_hint(&prop(0, 0, b"")));
        assert_eq!(motif_decorations(&prop32(400, &[MWM_HINTS_DECORATIONS, 0, 0, 0, 0])), Some(false));
        assert_eq!(motif_decorations(&prop32(400, &[MWM_HINTS_DECORATIONS, 0, 1, 0, 0])), Some(true));
        assert_eq!(motif_decorations(&prop32(400, &[1, 0, 0, 0, 0])), None, "only functions");
        assert_eq!(motif_decorations(&prop(0, 0, b"")), None);
    }

    #[test]
    fn size_hints_limit_sizes() {
        let mut v = vec![P_MIN_SIZE | P_MAX_SIZE, 0, 0, 0, 0, 100, 50, 400, 300];
        v.resize(18, 0);
        let h = SizeHints::parse(&prop32(41, &v));
        assert_eq!(h, SizeHints { min: Some((100, 50)), max: Some((400, 300)) });
        assert_eq!(h.constrain(10, 1000), (100, 300));
        assert_eq!(h.constrain(200, 100), (200, 100));
        assert_eq!(SizeHints::parse(&prop32(41, &[P_MIN_SIZE, 0, 0])), SizeHints::default(), "too short");
        assert_eq!(SizeHints::default().constrain(0, 0), (1, 1));
    }
}
