//! Window placement for the shell (ADR 0117): snap zones, their
//! rectangles, and what Super+arrow does, re-created from Alpine Shell's
//! tiling (its zones `left`, `right`, the four quarters and `full`, an
//! 8-pixel gap, no margin, and its arrow transitions; values only).
//!
//! - **Dragging** a window's title bar with the pointer at the work area's
//!   left or right edge snaps it to that half; near a corner (the top or
//!   bottom quarter of the edge), to that quarter; at the top (onto the
//!   ridge) it maximizes. The shell shows the zone while the button is
//!   held and applies it on release.
//! - **Super+arrows** move the active window between the zones: Left and
//!   Right to the halves (a quarter on the other side keeps its row), Up
//!   from a half or a bottom quarter to the top quarter and from anything
//!   else to maximized, Down from a half or a top quarter to the bottom
//!   quarter, from maximized back to floating and from anything else to
//!   minimized.

use poly_wayland::compose::Rect;

/// Between tiled windows, and half of it at the work area's edges
/// (Alpine Shell's `gap: 8`, `margin: 0`).
pub const GAP: i32 = 8;
/// How close (logical pixels) the pointer must be to an edge to snap.
pub const EDGE: i32 = 6;

/// Where a window can be snapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Maximized.
    Full,
}

impl Zone {
    /// The zone's share of the work area: (x, y, width, height) in
    /// halves.
    fn halves(self) -> (i32, i32, i32, i32) {
        match self {
            Zone::Left => (0, 0, 1, 2),
            Zone::Right => (1, 0, 1, 2),
            Zone::TopLeft => (0, 0, 1, 1),
            Zone::TopRight => (1, 0, 1, 1),
            Zone::BottomLeft => (0, 1, 1, 1),
            Zone::BottomRight => (1, 1, 1, 1),
            Zone::Full => (0, 0, 2, 2),
        }
    }

    pub const ALL: [Zone; 7] =
        [Zone::Left, Zone::Right, Zone::TopLeft, Zone::TopRight, Zone::BottomLeft, Zone::BottomRight, Zone::Full];
}

/// The frame's outer rectangle for `zone` in `work` (logical): the zone's
/// share less half the gap on every side.
pub fn zone_rect(zone: Zone, work: Rect) -> Rect {
    let (x, y, w, h) = zone.halves();
    let left = work.x + work.width * x / 2;
    let top = work.y + work.height * y / 2;
    let right = work.x + work.width * (x + w) / 2;
    let bottom = work.y + work.height * (y + h) / 2;
    let half = GAP / 2;
    Rect::new(left + half, top + half, right - left - GAP, bottom - top - GAP)
}

/// The zone a window dragged with the pointer at (`x`, `y`) would snap
/// to, in `work` (the ridge is above it).
pub fn zone_at(x: i32, y: i32, work: Rect) -> Option<Zone> {
    let corner = work.height / 4;
    let (top, bottom) = (y < work.y + corner, y >= work.bottom() - corner);
    if y < work.y + EDGE {
        // On the ridge or the work area's top edge: a corner there picks
        // the top quarter, the rest maximizes.
        return Some(if x < work.x + EDGE {
            Zone::TopLeft
        } else if x >= work.right() - EDGE {
            Zone::TopRight
        } else {
            Zone::Full
        });
    }
    if x < work.x + EDGE {
        return Some(if top {
            Zone::TopLeft
        } else if bottom {
            Zone::BottomLeft
        } else {
            Zone::Left
        });
    }
    if x >= work.right() - EDGE {
        return Some(if top {
            Zone::TopRight
        } else if bottom {
            Zone::BottomRight
        } else {
            Zone::Right
        });
    }
    None
}

/// The zone a tiled window is in, from its frame rectangle.
pub fn zone_of(tiled: Rect, work: Rect) -> Option<Zone> {
    Zone::ALL.into_iter().find(|&z| zone_rect(z, work) == tiled)
}

/// An arrow key with Super.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// What Super+arrow does to a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Tile(Zone),
    Maximize,
    Restore,
    Minimize,
}

/// The window's state before the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placed {
    Floating,
    Maximized,
    Tiled(Zone),
}

/// What Super+`arrow` does to a window `placed` so (Alpine Shell's
/// transitions).
pub fn arrow(placed: Placed, arrow: Arrow) -> Action {
    use Zone::*;
    let tiled = match placed {
        Placed::Tiled(z) => Some(z),
        _ => None,
    };
    match arrow {
        Arrow::Left => Action::Tile(match tiled {
            Some(TopRight) => TopLeft,
            Some(BottomRight) => BottomLeft,
            _ => Left,
        }),
        Arrow::Right => Action::Tile(match tiled {
            Some(TopLeft) => TopRight,
            Some(BottomLeft) => BottomRight,
            _ => Right,
        }),
        Arrow::Up => match tiled {
            Some(Left | BottomLeft) => Action::Tile(TopLeft),
            Some(Right | BottomRight) => Action::Tile(TopRight),
            _ => Action::Maximize,
        },
        Arrow::Down => match (placed, tiled) {
            (_, Some(Left | TopLeft)) => Action::Tile(BottomLeft),
            (_, Some(Right | TopRight)) => Action::Tile(BottomRight),
            (Placed::Maximized, _) | (_, Some(Full)) => Action::Restore,
            _ => Action::Minimize,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1280×800 with the 24-pixel ridge.
    const WORK: Rect = Rect::new(0, 24, 1280, 776);

    #[test]
    fn zone_rectangles_leave_the_gap() {
        assert_eq!(zone_rect(Zone::Left, WORK), Rect::new(4, 28, 632, 768));
        assert_eq!(zone_rect(Zone::Right, WORK), Rect::new(644, 28, 632, 768));
        assert_eq!(zone_rect(Zone::TopRight, WORK), Rect::new(644, 28, 632, 380));
        assert_eq!(zone_rect(Zone::BottomLeft, WORK), Rect::new(4, 416, 632, 380));
        assert_eq!(zone_rect(Zone::Full, WORK), Rect::new(4, 28, 1272, 768));
        // Two halves side by side: 8 pixels apart, 4 from the edges.
        let (l, r) = (zone_rect(Zone::Left, WORK), zone_rect(Zone::Right, WORK));
        assert_eq!(r.x - l.right(), GAP);
        assert_eq!(1280 - r.right(), GAP / 2);
        for z in Zone::ALL {
            assert_eq!(zone_of(zone_rect(z, WORK), WORK), Some(z));
        }
        assert_eq!(zone_of(Rect::new(1, 2, 3, 4), WORK), None);
    }

    #[test]
    fn dragging_to_the_edges() {
        assert_eq!(zone_at(640, 400, WORK), None, "the middle");
        assert_eq!(zone_at(2, 400, WORK), Some(Zone::Left));
        assert_eq!(zone_at(1279, 400, WORK), Some(Zone::Right));
        assert_eq!(zone_at(0, 100, WORK), Some(Zone::TopLeft), "the top quarter of the edge");
        assert_eq!(zone_at(1279, 790, WORK), Some(Zone::BottomRight));
        assert_eq!(zone_at(640, 10, WORK), Some(Zone::Full), "onto the ridge");
        assert_eq!(zone_at(640, 27, WORK), Some(Zone::Full), "the work area's top edge");
        assert_eq!(zone_at(640, 30, WORK), None);
        assert_eq!(zone_at(1279, 5, WORK), Some(Zone::TopRight));
    }

    #[test]
    fn super_arrows() {
        use Placed::*;
        assert_eq!(arrow(Floating, Arrow::Left), Action::Tile(Zone::Left));
        assert_eq!(arrow(Tiled(Zone::Left), Arrow::Right), Action::Tile(Zone::Right));
        assert_eq!(arrow(Tiled(Zone::TopRight), Arrow::Left), Action::Tile(Zone::TopLeft));
        assert_eq!(arrow(Tiled(Zone::Left), Arrow::Up), Action::Tile(Zone::TopLeft));
        assert_eq!(arrow(Tiled(Zone::BottomRight), Arrow::Up), Action::Tile(Zone::TopRight));
        assert_eq!(arrow(Floating, Arrow::Up), Action::Maximize);
        assert_eq!(arrow(Tiled(Zone::TopLeft), Arrow::Up), Action::Maximize);
        assert_eq!(arrow(Tiled(Zone::Right), Arrow::Down), Action::Tile(Zone::BottomRight));
        assert_eq!(arrow(Maximized, Arrow::Down), Action::Restore);
        assert_eq!(arrow(Floating, Arrow::Down), Action::Minimize);
        assert_eq!(arrow(Maximized, Arrow::Left), Action::Tile(Zone::Left));
        assert_eq!(arrow(Tiled(Zone::TopLeft), Arrow::Left), Action::Tile(Zone::Left));
        assert_eq!(arrow(Tiled(Zone::BottomLeft), Arrow::Down), Action::Minimize);
    }
}
