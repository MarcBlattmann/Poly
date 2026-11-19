//! The pointer's named looks besides the arrow (ADR 0234): the hand over links, the I-beam over
//! text, the resize arrows at window edges and the rest of what apps ask for by name
//! (`wp_cursor_shape_device_v1` shapes, Windows' standard `IDC_*` cursors).
//!
//! Each look is a small picture in logical pixels, drawn like Poly's arrow: black, white and
//! clear cells, each as large as the output's scale makes a logical pixel. The hand and the
//! hourglass are drawn cell by cell; the arrows, the I-beam, the crosshair and the "not allowed"
//! sign are made from their geometry ([`NamedCursor::cell`]), so they are symmetric.

/// A named pointer look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NamedCursor {
    /// Poly's arrow (`CursorLook::Arrow`).
    Arrow,
    /// The arrow with the busy spinner (`CursorLook::Busy`): working in the background.
    Progress,
    /// A pointing hand: a link.
    Hand,
    /// The I-beam: text.
    Text,
    /// An hourglass: busy, wait.
    Wait,
    Crosshair,
    /// Four arrows: move, scroll in every direction.
    Move,
    /// A crossed-out circle.
    NotAllowed,
    /// Left and right.
    ResizeEW,
    /// Up and down.
    ResizeNS,
    /// Top-left and bottom-right.
    ResizeNWSE,
    /// Top-right and bottom-left.
    ResizeNESW,
}

/// What a cell of a look is.
pub const CLEAR: u8 = 0;
pub const BLACK: u8 = 1;
pub const WHITE: u8 = 2;

const HAND: [&[u8; 16]; 19] = [
    b"     XX         ",
    b"    X..X        ",
    b"    X..X        ",
    b"    X..X        ",
    b"    X..X        ",
    b"    X..XXX      ",
    b"    X..X..XXX   ",
    b"    X..X..X..XX ",
    b" XX X..X..X..X.X",
    b"X..XX..........X",
    b"X...X..........X",
    b" X.............X",
    b"  X............X",
    b"  X...........X ",
    b"   X..........X ",
    b"   X.........X  ",
    b"    X........X  ",
    b"    X........X  ",
    b"    XXXXXXXXXX  ",
];

const HOURGLASS: [&[u8; 11]; 15] = [
    b"XXXXXXXXXXX",
    b" X.......X ",
    b" X.......X ",
    b" X.XXXXX.X ",
    b"  X.XXX.X  ",
    b"   X.X.X   ",
    b"    X.X    ",
    b"    X.X    ",
    b"   X...X   ",
    b"  X..X..X  ",
    b" X...X...X ",
    b" X..XXX..X ",
    b" X.XXXXX.X ",
    b" X.......X ",
    b"XXXXXXXXXXX",
];

fn art(rows: &[&[u8]], x: i32, y: i32) -> u8 {
    match rows.get(y as usize).and_then(|r| r.get(x as usize)) {
        Some(b'X') => BLACK,
        Some(b'.') => WHITE,
        _ => CLEAR,
    }
}

/// A double arrow along x, `len` cells long and 9 high (tips at x = 0 and x = len - 1, y = 4):
/// heads of 5 cells, a shaft of one white cell between black ones.
fn double_arrow(len: i32, x: i32, y: i32) -> u8 {
    if !(0..len).contains(&x) || !(0..9).contains(&y) {
        return CLEAR;
    }
    let dy = (y - 4).abs();
    let d = x.min(len - 1 - x);
    if d <= 4 {
        if dy > d {
            CLEAR
        } else if dy == d || (d == 4 && dy >= 2) {
            BLACK
        } else {
            WHITE
        }
    } else if dy == 0 {
        WHITE
    } else if dy == 1 {
        BLACK
    } else {
        CLEAR
    }
}

/// A double arrow from (0, 0) to (14, 14): triangular heads in the corners, a shaft along the
/// diagonal.
fn diagonal_arrow(x: i32, y: i32) -> u8 {
    if !(0..15).contains(&x) || !(0..15).contains(&y) {
        return CLEAR;
    }
    let a = (x - y).abs();
    let s = x + y;
    if a <= 1 && (3..=25).contains(&s) {
        return WHITE;
    }
    let head = |hx: i32, hy: i32| -> Option<u8> {
        (hx + hy <= 6).then(|| if hx == 0 || hy == 0 || hx + hy == 6 { BLACK } else { WHITE })
    };
    if let Some(c) = head(x, y).or_else(|| head(14 - x, 14 - y)) {
        return c;
    }
    if a == 2 && (4..=24).contains(&s) { BLACK } else { CLEAR }
}

/// A black figure with a white edge one cell wide around it, in a `w` x `h` box.
fn haloed(w: i32, h: i32, x: i32, y: i32, core: impl Fn(i32, i32) -> bool) -> u8 {
    if !(0..w).contains(&x) || !(0..h).contains(&y) {
        return CLEAR;
    }
    if core(x, y) {
        return BLACK;
    }
    let near = (-1..=1).any(|dy| (-1..=1).any(|dx| core(x + dx, y + dy)));
    if near { WHITE } else { CLEAR }
}

impl NamedCursor {
    /// The picture's size in logical pixels.
    pub fn size(self) -> (i32, i32) {
        match self {
            NamedCursor::Arrow | NamedCursor::Progress => super::CURSOR_SIZE,
            NamedCursor::Hand => (16, 19),
            NamedCursor::Text => (7, 19),
            NamedCursor::Wait => (11, 15),
            NamedCursor::Crosshair => (17, 17),
            NamedCursor::Move => (19, 19),
            NamedCursor::NotAllowed => (17, 17),
            NamedCursor::ResizeEW => (19, 9),
            NamedCursor::ResizeNS => (9, 19),
            NamedCursor::ResizeNWSE | NamedCursor::ResizeNESW => (15, 15),
        }
    }

    /// The hot spot: the cell that is at the pointer's position.
    pub fn hotspot(self) -> (i32, i32) {
        match self {
            NamedCursor::Arrow | NamedCursor::Progress => (0, 0),
            NamedCursor::Hand => (5, 0),
            NamedCursor::Text => (3, 9),
            NamedCursor::Wait => (5, 7),
            NamedCursor::Crosshair | NamedCursor::NotAllowed => (8, 8),
            NamedCursor::Move => (9, 9),
            NamedCursor::ResizeEW => (9, 4),
            NamedCursor::ResizeNS => (4, 9),
            NamedCursor::ResizeNWSE | NamedCursor::ResizeNESW => (7, 7),
        }
    }

    /// The cell at (`x`, `y`) of the picture: [`CLEAR`], [`BLACK`] or [`WHITE`]. The arrow and
    /// the progress look are drawn by the scene's own arrow and are clear here.
    pub fn cell(self, x: i32, y: i32) -> u8 {
        match self {
            NamedCursor::Arrow | NamedCursor::Progress => CLEAR,
            NamedCursor::Hand => art(&HAND.map(|r| &r[..]), x, y),
            NamedCursor::Wait => art(&HOURGLASS.map(|r| &r[..]), x, y),
            NamedCursor::Text => haloed(7, 19, x, y, |x, y| {
                (x == 3 && (2..=16).contains(&y)) || ((y == 1 || y == 17) && matches!(x, 1 | 2 | 4 | 5))
            }),
            NamedCursor::Crosshair => {
                haloed(17, 17, x, y, |x, y| (x == 8 && (1..=15).contains(&y)) || (y == 8 && (1..=15).contains(&x)))
            }
            NamedCursor::NotAllowed => haloed(17, 17, x, y, |x, y| {
                let (dx, dy) = (x - 8, y - 8);
                let d2 = dx * dx + dy * dy;
                (30..=56).contains(&d2) || (d2 < 30 && (dx - dy).abs() <= 1 && dx.abs() <= 5)
            }),
            NamedCursor::ResizeEW => double_arrow(19, x, y),
            NamedCursor::ResizeNS => double_arrow(19, y, x),
            NamedCursor::ResizeNWSE => diagonal_arrow(x, y),
            NamedCursor::ResizeNESW => diagonal_arrow(14 - x, y),
            NamedCursor::Move => {
                // Both double arrows, crossing in the middle (white wins where they meet).
                let a = double_arrow(19, x, y - 5);
                let b = double_arrow(19, y, x - 5);
                a.max(b)
            }
        }
    }

    /// The look of a `wp_cursor_shape_device_v1` shape (cursor-shape-v1.xml): the names that
    /// mean the same picture share one; the arrow for those Poly has no picture of.
    pub fn of_shape(shape: u32) -> NamedCursor {
        match shape {
            0x4 => NamedCursor::Hand,
            0x5 => NamedCursor::Progress,
            0x6 => NamedCursor::Wait,
            0x7 | 0x8 => NamedCursor::Crosshair,
            0x9 | 0xa => NamedCursor::Text,
            0xd | 0x10 | 0x11 | 0x20 | 0x24 => NamedCursor::Move,
            0xe | 0xf => NamedCursor::NotAllowed,
            0x12 | 0x19 | 0x1a | 0x1e => NamedCursor::ResizeEW,
            0x13 | 0x16 | 0x1b | 0x1f => NamedCursor::ResizeNS,
            0x15 | 0x17 | 0x1d => NamedCursor::ResizeNWSE,
            0x14 | 0x18 | 0x1c => NamedCursor::ResizeNESW,
            _ => NamedCursor::Arrow,
        }
    }

    /// The look of a Windows standard cursor, its `IDC_*` resource number (winuser.h); the
    /// arrow for the others.
    pub fn of_windows(idc: u16) -> NamedCursor {
        match idc {
            32513 => NamedCursor::Text,       // IDC_IBEAM
            32514 => NamedCursor::Wait,       // IDC_WAIT
            32515 => NamedCursor::Crosshair,  // IDC_CROSS
            32642 => NamedCursor::ResizeNWSE, // IDC_SIZENWSE
            32643 => NamedCursor::ResizeNESW, // IDC_SIZENESW
            32644 => NamedCursor::ResizeEW,   // IDC_SIZEWE
            32645 => NamedCursor::ResizeNS,   // IDC_SIZENS
            32646 => NamedCursor::Move,       // IDC_SIZEALL
            32648 => NamedCursor::NotAllowed, // IDC_NO
            32649 => NamedCursor::Hand,       // IDC_HAND
            32650 => NamedCursor::Progress,   // IDC_APPSTARTING
            _ => NamedCursor::Arrow,
        }
    }

    /// The look for resizing by `edges` (xdg_toplevel's `resize_edge` bits: top 1, bottom 2,
    /// left 4, right 8); `None` for no edge.
    pub fn of_edges(edges: u32) -> Option<NamedCursor> {
        let (v, h) = (edges & 3, edges & 12);
        Some(match (v, h) {
            (0, 0) => return None,
            (_, 0) => NamedCursor::ResizeNS,
            (0, _) => NamedCursor::ResizeEW,
            (1, 4) | (2, 8) => NamedCursor::ResizeNWSE,
            _ => NamedCursor::ResizeNESW,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;

    fn picture(c: NamedCursor) -> Vec<String> {
        let (w, h) = c.size();
        (0..h).map(|y| (0..w).map(|x| [' ', 'X', '.'][c.cell(x, y) as usize]).collect()).collect()
    }

    #[test]
    fn every_picture_has_its_hot_spot_inside_and_cells_only_inside_its_size() {
        for c in [
            NamedCursor::Hand,
            NamedCursor::Text,
            NamedCursor::Wait,
            NamedCursor::Crosshair,
            NamedCursor::Move,
            NamedCursor::NotAllowed,
            NamedCursor::ResizeEW,
            NamedCursor::ResizeNS,
            NamedCursor::ResizeNWSE,
            NamedCursor::ResizeNESW,
        ] {
            let ((w, h), (hx, hy)) = (c.size(), c.hotspot());
            assert!((0..w).contains(&hx) && (0..h).contains(&hy), "{c:?}");
            assert_eq!(c.cell(-1, 0), CLEAR);
            assert_eq!(c.cell(w, h - 1), CLEAR);
            assert!(picture(c).iter().any(|r| r.contains('X')), "{c:?}");
        }
    }

    #[test]
    fn the_arrows_are_symmetric_and_white_inside_a_black_edge() {
        assert_eq!(
            picture(NamedCursor::ResizeEW),
            [
                "    X         X    ",
                "   XX         XX   ",
                "  X.X         X.X  ",
                " X...XXXXXXXXX...X ",
                "X.................X",
                " X...XXXXXXXXX...X ",
                "  X.X         X.X  ",
                "   XX         XX   ",
                "    X         X    ",
            ]
        );
        // NS is EW turned, NESW is NWSE mirrored; the move arrows hold both double arrows.
        for x in 0..19 {
            for y in 0..9 {
                assert_eq!(NamedCursor::ResizeNS.cell(y, x), NamedCursor::ResizeEW.cell(x, y));
            }
        }
        for x in 0..15 {
            for y in 0..15 {
                let d = NamedCursor::ResizeNWSE.cell(x, y);
                assert_eq!(d, NamedCursor::ResizeNWSE.cell(14 - x, 14 - y), "point symmetric");
                assert_eq!(d, NamedCursor::ResizeNWSE.cell(y, x), "symmetric about the diagonal");
                assert_eq!(NamedCursor::ResizeNESW.cell(14 - x, y), d);
            }
        }
        assert_eq!(NamedCursor::ResizeNWSE.cell(0, 0), BLACK, "a black tip");
        assert_eq!(NamedCursor::ResizeNWSE.cell(7, 7), WHITE);
        assert_eq!(NamedCursor::Move.cell(9, 9), WHITE);
        assert_eq!(NamedCursor::Move.cell(9, 0), BLACK);
        assert_eq!(NamedCursor::Move.cell(0, 9), BLACK);
    }

    #[test]
    fn the_i_beam_is_black_with_a_white_edge() {
        let p = picture(NamedCursor::Text);
        assert_eq!(p[0], ".......");
        assert_eq!(p[1], ".XX.XX.");
        assert_eq!(p[9], "  .X.  ");
        assert_eq!(p[18], ".......");
        assert_eq!(NamedCursor::Text.cell(3, 9), BLACK, "the hot spot is on the stem");
    }

    #[test]
    fn names_map_to_pictures() {
        assert_eq!(NamedCursor::of_shape(0x4), NamedCursor::Hand);
        assert_eq!(NamedCursor::of_shape(0x9), NamedCursor::Text);
        assert_eq!(NamedCursor::of_shape(0x1a), NamedCursor::ResizeEW);
        assert_eq!(NamedCursor::of_shape(0x17), NamedCursor::ResizeNWSE);
        assert_eq!(NamedCursor::of_shape(0x1), NamedCursor::Arrow);
        assert_eq!(NamedCursor::of_windows(32649), NamedCursor::Hand);
        assert_eq!(NamedCursor::of_windows(32513), NamedCursor::Text);
        assert_eq!(NamedCursor::of_windows(32645), NamedCursor::ResizeNS);
        assert_eq!(NamedCursor::of_windows(32512), NamedCursor::Arrow);
        assert_eq!(NamedCursor::of_edges(0), None);
        assert_eq!(NamedCursor::of_edges(4), Some(NamedCursor::ResizeEW));
        assert_eq!(NamedCursor::of_edges(2), Some(NamedCursor::ResizeNS));
        assert_eq!(NamedCursor::of_edges(1 | 4), Some(NamedCursor::ResizeNWSE));
        assert_eq!(NamedCursor::of_edges(2 | 8), Some(NamedCursor::ResizeNWSE));
        assert_eq!(NamedCursor::of_edges(1 | 8), Some(NamedCursor::ResizeNESW));
        assert_eq!(NamedCursor::of_edges(2 | 4), Some(NamedCursor::ResizeNESW));
    }
}
