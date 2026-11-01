//! Atoms: the predefined ones ("X Window System Protocol", appendix B,
//! "Predefined Atoms") have fixed values; every other one the window
//! manager uses is interned by name at start ([`Atoms::NAMES`]).

use crate::Xid;

/// Predefined atoms.
pub mod predefined {
    use crate::Xid;
    pub const PRIMARY: Xid = 1;
    pub const SECONDARY: Xid = 2;
    pub const ATOM: Xid = 4;
    pub const CARDINAL: Xid = 6;
    pub const INTEGER: Xid = 19;
    pub const STRING: Xid = 31;
    pub const WINDOW: Xid = 33;
    pub const WM_COMMAND: Xid = 34;
    pub const WM_HINTS: Xid = 35;
    pub const WM_CLIENT_MACHINE: Xid = 36;
    pub const WM_ICON_NAME: Xid = 37;
    pub const WM_NAME: Xid = 39;
    pub const WM_NORMAL_HINTS: Xid = 40;
    pub const WM_SIZE_HINTS: Xid = 41;
    pub const WM_CLASS: Xid = 67;
    pub const WM_TRANSIENT_FOR: Xid = 68;
}

macro_rules! atoms {
    ($($field:ident = $name:literal,)*) => {
        /// The interned atoms the window manager uses, by name.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        #[allow(non_snake_case)]
        pub struct Atoms {
            $(pub $field: Xid,)*
        }

        impl Atoms {
            /// Every name, in the order of the fields.
            pub const NAMES: &'static [&'static str] = &[$($name,)*];

            /// Set the atom of the `i`th name.
            pub fn set(&mut self, i: usize, atom: Xid) {
                let mut k = 0;
                $(
                    if k == i {
                        self.$field = atom;
                    }
                    k += 1;
                )*
                let _ = k;
            }

            /// The name of an interned atom, if it is one of these.
            pub fn name(&self, atom: Xid) -> Option<&'static str> {
                if atom == 0 {
                    return None;
                }
                $(
                    if self.$field == atom {
                        return Some($name);
                    }
                )*
                None
            }
        }
    };
}

atoms! {
    WM_PROTOCOLS = "WM_PROTOCOLS",
    WM_DELETE_WINDOW = "WM_DELETE_WINDOW",
    WM_TAKE_FOCUS = "WM_TAKE_FOCUS",
    WM_STATE = "WM_STATE",
    WM_CHANGE_STATE = "WM_CHANGE_STATE",
    WM_S0 = "WM_S0",
    UTF8_STRING = "UTF8_STRING",
    COMPOUND_TEXT = "COMPOUND_TEXT",
    TARGETS = "TARGETS",
    _NET_SUPPORTED = "_NET_SUPPORTED",
    _NET_SUPPORTING_WM_CHECK = "_NET_SUPPORTING_WM_CHECK",
    _NET_CLIENT_LIST = "_NET_CLIENT_LIST",
    _NET_ACTIVE_WINDOW = "_NET_ACTIVE_WINDOW",
    _NET_WM_NAME = "_NET_WM_NAME",
    _NET_WM_PID = "_NET_WM_PID",
    _NET_WM_MOVERESIZE = "_NET_WM_MOVERESIZE",
    _NET_FRAME_EXTENTS = "_NET_FRAME_EXTENTS",
    _NET_WM_STATE = "_NET_WM_STATE",
    _NET_WM_STATE_FOCUSED = "_NET_WM_STATE_FOCUSED",
    _NET_WM_STATE_MODAL = "_NET_WM_STATE_MODAL",
    _NET_WM_WINDOW_TYPE = "_NET_WM_WINDOW_TYPE",
    _NET_WM_WINDOW_TYPE_NORMAL = "_NET_WM_WINDOW_TYPE_NORMAL",
    _NET_WM_WINDOW_TYPE_DIALOG = "_NET_WM_WINDOW_TYPE_DIALOG",
    _NET_WM_WINDOW_TYPE_UTILITY = "_NET_WM_WINDOW_TYPE_UTILITY",
    _NET_WM_WINDOW_TYPE_TOOLBAR = "_NET_WM_WINDOW_TYPE_TOOLBAR",
    _NET_WM_WINDOW_TYPE_SPLASH = "_NET_WM_WINDOW_TYPE_SPLASH",
    _NET_WM_WINDOW_TYPE_DOCK = "_NET_WM_WINDOW_TYPE_DOCK",
    _NET_WM_WINDOW_TYPE_DESKTOP = "_NET_WM_WINDOW_TYPE_DESKTOP",
    _NET_WM_WINDOW_TYPE_MENU = "_NET_WM_WINDOW_TYPE_MENU",
    _NET_WM_WINDOW_TYPE_DROPDOWN_MENU = "_NET_WM_WINDOW_TYPE_DROPDOWN_MENU",
    _NET_WM_WINDOW_TYPE_POPUP_MENU = "_NET_WM_WINDOW_TYPE_POPUP_MENU",
    _NET_WM_WINDOW_TYPE_TOOLTIP = "_NET_WM_WINDOW_TYPE_TOOLTIP",
    _NET_WM_WINDOW_TYPE_NOTIFICATION = "_NET_WM_WINDOW_TYPE_NOTIFICATION",
    _NET_WM_WINDOW_TYPE_COMBO = "_NET_WM_WINDOW_TYPE_COMBO",
    _NET_WM_WINDOW_TYPE_DND = "_NET_WM_WINDOW_TYPE_DND",
    _NET_WM_CM_S0 = "_NET_WM_CM_S0",
    _MOTIF_WM_HINTS = "_MOTIF_WM_HINTS",
    WL_SURFACE_ID = "WL_SURFACE_ID",
    WL_SURFACE_SERIAL = "WL_SURFACE_SERIAL",
}

impl Atoms {
    /// The window types that get Poly's frame (EWMH: normal top-level
    /// windows, dialogs and utility windows; a window without the
    /// property is normal, or a dialog if it is transient).
    pub fn framed_types(&self) -> [Xid; 3] {
        [self._NET_WM_WINDOW_TYPE_NORMAL, self._NET_WM_WINDOW_TYPE_DIALOG, self._NET_WM_WINDOW_TYPE_UTILITY]
    }

    /// What `_NET_SUPPORTED` lists.
    pub fn supported(&self) -> [Xid; 13] {
        [
            self._NET_SUPPORTED,
            self._NET_SUPPORTING_WM_CHECK,
            self._NET_CLIENT_LIST,
            self._NET_ACTIVE_WINDOW,
            self._NET_WM_NAME,
            self._NET_WM_MOVERESIZE,
            self._NET_FRAME_EXTENTS,
            self._NET_WM_STATE,
            self._NET_WM_STATE_FOCUSED,
            self._NET_WM_WINDOW_TYPE,
            self._NET_WM_WINDOW_TYPE_NORMAL,
            self._NET_WM_WINDOW_TYPE_DIALOG,
            self._NET_WM_WINDOW_TYPE_UTILITY,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_fields_line_up() {
        let mut a = Atoms::default();
        for i in 0..Atoms::NAMES.len() {
            a.set(i, 300 + i as Xid);
        }
        for (i, name) in Atoms::NAMES.iter().enumerate() {
            assert_eq!(a.name(300 + i as Xid), Some(*name));
        }
        assert_eq!(a.WM_PROTOCOLS, 300);
        assert_eq!(a.WL_SURFACE_SERIAL, 300 + Atoms::NAMES.len() as Xid - 1);
        assert_eq!(a.name(0), None);
        assert_eq!(a.name(7), None);
        // Every name once.
        for (i, n) in Atoms::NAMES.iter().enumerate() {
            assert!(!Atoms::NAMES[i + 1..].contains(n), "{n}");
        }
    }
}
