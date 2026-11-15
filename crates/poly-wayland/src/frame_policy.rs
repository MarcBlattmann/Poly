//! Who draws a toplevel's frame: Poly or the client (ADR 0183, refining
//! ADR 0144).
//!
//! A window must have one title bar and one set of window buttons. Poly's
//! frame is the default (ADR 0106); a client that draws its own title bar
//! (a GTK header bar, Chromium's tab strip) tells the compositor so in one
//! of the ways toolkits do, and gets no Poly frame. This module only
//! decides; [`crate::server`] collects the signals from the protocols and
//! applies the answer.
//!
//! The signals, strongest first:
//!
//! 1. **xdg-decoration** (`set_mode`): GTK 4, Qt 5 and 6 and Chromium say
//!    what they want and the compositor answers (the answer it sends is the
//!    one decided here, so the client follows it).
//! 2. **KDE's server decoration protocol**, which GTK 3 and 4 use: a client
//!    that makes a decoration object for a surface gets the mode it asks for
//!    (`server`: Poly's frame; `client` or `none`: its own), or the default
//!    ([`KDE_DEFAULT`]) if it asks for none; a client that bound the manager
//!    but made no object for this surface draws its own frame.
//! 3. **Shadow margins**: a toplevel whose window geometry sits inside its
//!    buffer on all four sides draws its own shadow, which only a client-drawn
//!    frame has. This catches toolkits and apps that use neither protocol.
//!    Once seen it stays (a maximized window drops its shadow but keeps its
//!    title bar).
//! 4. Otherwise Poly's frame.

/// What a client asked for through `zxdg_toplevel_decoration_v1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ask {
    /// No decoration object, or `unset_mode`.
    #[default]
    Silent,
    /// `set_mode(client_side)`.
    Client,
    /// `set_mode(server_side)`.
    Server,
}

/// `org_kde_kwin_server_decoration` modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KdeMode {
    None,
    Client,
    Server,
}

impl KdeMode {
    /// The mode of the wire value, if it is one.
    pub fn from_wire(v: u32) -> Option<KdeMode> {
        match v {
            0 => Some(KdeMode::None),
            1 => Some(KdeMode::Client),
            2 => Some(KdeMode::Server),
            _ => None,
        }
    }

    pub fn wire(self) -> u32 {
        match self {
            KdeMode::None => 0,
            KdeMode::Client => 1,
            KdeMode::Server => 2,
        }
    }
}

/// The mode the server decoration manager announces and uses for an object
/// that asked for none. Not `Server`, though Poly prefers its own frame:
/// GTK 3.24 takes a path to its window size with `server` that is sometimes
/// garbage under Poly (ADR 0183).
pub const KDE_DEFAULT: KdeMode = KdeMode::Client;

/// What the client did with KDE's server decoration protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kde {
    /// It never bound the manager.
    #[default]
    NotBound,
    /// It bound the manager but made no object for this surface.
    NoObject,
    /// It made an object for this surface, and asked for `requested`, if it
    /// did.
    Object { requested: Option<KdeMode> },
}

/// Everything known about one toplevel's decoration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signals {
    pub xdg: Ask,
    pub kde: Kde,
    /// The window geometry has been seen inside the buffer on all four
    /// sides ([`draws_shadow`]).
    pub shadow: bool,
}

/// Who draws the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Poly,
    Client,
}

/// Why, for the log and the tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    XdgClient,
    XdgServer,
    KdeServer,
    KdeClient,
    KdeNoObject,
    Shadow,
    Default,
}

impl Reason {
    pub fn name(self) -> &'static str {
        match self {
            Reason::XdgClient => "xdg-decoration: client_side",
            Reason::XdgServer => "xdg-decoration: server_side",
            Reason::KdeServer => "server decoration: the server's frame",
            Reason::KdeClient => "server decoration: the client's frame",
            Reason::KdeNoObject => "server decoration bound, no object for the surface",
            Reason::Shadow => "its own shadow margins",
            Reason::Default => "no signal",
        }
    }
}

/// Decide who draws the frame.
pub fn decide(s: &Signals) -> (Frame, Reason) {
    match s.xdg {
        Ask::Client => return (Frame::Client, Reason::XdgClient),
        Ask::Server => return (Frame::Poly, Reason::XdgServer),
        Ask::Silent => {}
    }
    match s.kde {
        Kde::Object { requested } => {
            return match requested.unwrap_or(KDE_DEFAULT) {
                KdeMode::Server => (Frame::Poly, Reason::KdeServer),
                KdeMode::Client | KdeMode::None => (Frame::Client, Reason::KdeClient),
            };
        }
        Kde::NoObject => return (Frame::Client, Reason::KdeNoObject),
        Kde::NotBound => {}
    }
    if s.shadow { (Frame::Client, Reason::Shadow) } else { (Frame::Poly, Reason::Default) }
}

/// The least a margin, and both margins of an axis together, may be to
/// count as a shadow (a one pixel crop is not one).
const MIN_MARGIN_SUM: i32 = 8;

/// Whether a toplevel of `buffer` (logical width and height) with window
/// geometry `geometry` (x, y, width, height) draws a shadow: there is room
/// around the geometry on every side.
pub fn draws_shadow(buffer: (i32, i32), geometry: (i32, i32, i32, i32)) -> bool {
    let (bw, bh) = buffer;
    let (x, y, w, h) = geometry;
    let (left, top, right, bottom) = (x, y, bw - x - w, bh - y - h);
    left > 0 && top > 0 && right > 0 && bottom > 0 && left + right >= MIN_MARGIN_SUM && top + bottom >= MIN_MARGIN_SUM
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(s: Signals) -> Frame {
        decide(&s).0
    }

    #[test]
    fn nothing_known_is_poly_s_frame() {
        assert_eq!(decide(&Signals::default()), (Frame::Poly, Reason::Default));
    }

    #[test]
    fn xdg_decoration_is_followed_and_outranks_the_rest() {
        let both = Signals { xdg: Ask::Client, kde: Kde::Object { requested: Some(KdeMode::Server) }, shadow: false };
        assert_eq!(decide(&both), (Frame::Client, Reason::XdgClient));
        let server = Signals { xdg: Ask::Server, kde: Kde::NoObject, shadow: true };
        assert_eq!(decide(&server), (Frame::Poly, Reason::XdgServer));
    }

    #[test]
    fn a_gtk3_window_with_a_header_bar_makes_no_decoration_object() {
        let s = Signals { kde: Kde::NoObject, ..Signals::default() };
        assert_eq!(decide(&s), (Frame::Client, Reason::KdeNoObject));
    }

    #[test]
    fn an_object_gets_the_mode_it_asks_for_or_the_default() {
        let asks = Signals { kde: Kde::Object { requested: Some(KdeMode::Server) }, ..Signals::default() };
        assert_eq!(decide(&asks), (Frame::Poly, Reason::KdeServer));
        let none = Signals { kde: Kde::Object { requested: None }, ..Signals::default() };
        assert_eq!(decide(&none), (Frame::Client, Reason::KdeClient), "the default is {KDE_DEFAULT:?}");
    }

    #[test]
    fn asking_the_server_decoration_for_client_or_none_leaves_the_frame_to_the_client() {
        for mode in [KdeMode::Client, KdeMode::None] {
            let s = Signals { kde: Kde::Object { requested: Some(mode) }, ..Signals::default() };
            assert_eq!(decide(&s), (Frame::Client, Reason::KdeClient));
        }
    }

    #[test]
    fn a_shadow_is_a_client_drawn_frame_when_no_protocol_said_otherwise() {
        let s = Signals { shadow: true, ..Signals::default() };
        assert_eq!(decide(&s), (Frame::Client, Reason::Shadow));
        let asked =
            Signals { kde: Kde::Object { requested: Some(KdeMode::Server) }, shadow: true, ..Signals::default() };
        assert_eq!(frame(asked), Frame::Poly, "a protocol's word outranks a guess");
    }

    #[test]
    fn shadow_margins_are_room_on_all_four_sides() {
        // GTK 3's: about 26 px around, a little less above.
        assert!(draws_shadow((560, 400), (26, 23, 508, 354)));
        // No margins: a maximized window, or an app without a shadow.
        assert!(!draws_shadow((560, 400), (0, 0, 560, 400)));
        // A geometry that crops one side only.
        assert!(!draws_shadow((560, 400), (0, 20, 560, 380)));
        assert!(!draws_shadow((560, 400), (20, 0, 540, 400)));
        // A one pixel border is not a shadow.
        assert!(!draws_shadow((560, 400), (1, 1, 558, 398)));
        // A geometry reaching out of the buffer is none.
        assert!(!draws_shadow((100, 100), (10, 10, 100, 100)));
    }

    #[test]
    fn modes_round_trip_through_the_wire() {
        for m in [KdeMode::None, KdeMode::Client, KdeMode::Server] {
            assert_eq!(KdeMode::from_wire(m.wire()), Some(m));
        }
        assert_eq!(KdeMode::from_wire(3), None);
    }
}
