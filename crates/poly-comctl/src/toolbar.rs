//! The toolbar's buttons and their layout (`ToolbarWindow32`; spec-win-apps-5
//! `specs/windows/comctl32.dll/ToolbarWindow32.yaml`, `CreateToolbarEx.yaml`; Learn, "Toolbar
//! Control Reference"). No window handling here: `comctl32.dll` decodes the messages
//! (`commctrl.h`) and paints.

use alloc::vec::Vec;

/// `TBSTATE_*` and `TBSTYLE_*` (`commctrl.h`).
pub const TBSTATE_CHECKED: u8 = 0x01;
pub const TBSTATE_PRESSED: u8 = 0x02;
pub const TBSTATE_ENABLED: u8 = 0x04;
pub const TBSTATE_HIDDEN: u8 = 0x08;
pub const TBSTATE_INDETERMINATE: u8 = 0x10;
pub const TBSTATE_WRAP: u8 = 0x20;
pub const TBSTYLE_BUTTON: u8 = 0x00;
pub const TBSTYLE_SEP: u8 = 0x01;
pub const TBSTYLE_CHECK: u8 = 0x02;
pub const TBSTYLE_GROUP: u8 = 0x04;
pub const TBSTYLE_CHECKGROUP: u8 = TBSTYLE_GROUP | TBSTYLE_CHECK;
pub const TBSTYLE_FLAT: u32 = 0x0800;
pub const TBSTYLE_WRAPABLE: u32 = 0x0200;
pub const CCS_NODIVIDER: u32 = 0x0040;

/// The padding a button has around its bitmap (`TB_GETPADDING` is 7 and 6) and the width of a
/// separator that was given none.
pub const PAD_X: i32 = 7;
pub const PAD_Y: i32 = 6;
pub const SEPARATOR_WIDTH: i32 = 8;
/// The bitmaps are 16 x 16 until `TB_SETBITMAPSIZE` says otherwise.
pub const DEFAULT_BITMAP: (i32, i32) = (16, 16);

/// A button (`TBBUTTON`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Button {
    pub image: i32,
    pub command: i32,
    pub state: u8,
    pub style: u8,
    pub data: usize,
    pub string: isize,
}

impl Button {
    pub fn is_separator(&self) -> bool {
        self.style & TBSTYLE_SEP != 0
    }

    pub fn hidden(&self) -> bool {
        self.state & TBSTATE_HIDDEN != 0
    }
}

/// A rectangle: left, top, right, bottom.
pub type Rect = [i32; 4];

/// The toolbar's state.
#[derive(Clone, Debug)]
pub struct Toolbar {
    pub buttons: Vec<Button>,
    /// The bitmaps `TB_ADDBITMAP` added: the handle (or resource id) and how many images it has.
    pub bitmaps: Vec<(usize, i32)>,
    pub button_size: (i32, i32),
    pub bitmap_size: (i32, i32),
    pub rows: i32,
    pub style: u32,
    pub ex_style: u32,
    pub struct_size: u32,
}

impl Default for Toolbar {
    fn default() -> Self {
        Toolbar {
            buttons: Vec::new(),
            bitmaps: Vec::new(),
            button_size: (DEFAULT_BITMAP.0 + PAD_X, DEFAULT_BITMAP.1 + PAD_Y),
            bitmap_size: DEFAULT_BITMAP,
            rows: 1,
            style: 0,
            ex_style: 0,
            struct_size: 0,
        }
    }
}

impl Toolbar {
    /// How many images the added bitmaps hold.
    pub fn image_count(&self) -> i32 {
        self.bitmaps.iter().map(|b| b.1).sum()
    }

    /// `TB_ADDBITMAP`: the index of the first image added.
    pub fn add_bitmap(&mut self, handle: usize, count: i32) -> i32 {
        let first = self.image_count();
        self.bitmaps.push((handle, count));
        first
    }

    /// `TB_SETBITMAPSIZE`: the buttons grow to hold the bitmaps.
    pub fn set_bitmap_size(&mut self, cx: i32, cy: i32) {
        self.bitmap_size = (cx, cy);
        self.button_size = (cx + PAD_X, cy + PAD_Y);
    }

    /// `TB_INSERTBUTTON`: the button goes before `index`; a negative index and one
    /// past the end append (`ToolbarWindow32.yaml`); the index it got.
    pub fn insert(&mut self, index: i32, b: Button) -> Option<usize> {
        // A negative index is a huge unsigned one: the button goes last.
        let at = if index < 0 { self.buttons.len() } else { (index as usize).min(self.buttons.len()) };
        self.buttons.insert(at, b);
        Some(at)
    }

    /// The index of the button with `command` (separators have none: `TB_COMMANDTOINDEX(0)` finds
    /// the first one), -1 when there is none.
    pub fn command_to_index(&self, command: i32) -> i32 {
        self.buttons.iter().position(|b| b.command == command).map_or(-1, |i| i as i32)
    }

    /// The top margin of the rows: a flat toolbar has none.
    pub fn top_margin(&self) -> i32 {
        if self.style & TBSTYLE_FLAT != 0 { 0 } else { 2 }
    }

    /// The divider the toolbar draws above its buttons (a non-client part): none for a flat
    /// toolbar and with `CCS_NODIVIDER`.
    pub fn divider(&self) -> i32 {
        if self.style & (TBSTYLE_FLAT | CCS_NODIVIDER) != 0 { 0 } else { 2 }
    }

    /// The rectangles of the buttons in client coordinates, rows included (`TBSTATE_WRAP` and
    /// `TBSTYLE_WRAPABLE` start a new row at `width`); a hidden button has none (all 0).
    pub fn layout(&self, width: i32) -> Vec<Rect> {
        let (bw, bh) = self.button_size;
        let top = self.top_margin();
        let wrapable = self.style & TBSTYLE_WRAPABLE != 0;
        let (mut x, mut row) = (0, 0);
        let mut out = Vec::with_capacity(self.buttons.len());
        for b in &self.buttons {
            if b.hidden() {
                out.push([0, 0, 0, 0]);
                continue;
            }
            let w = if b.is_separator() { if b.image > 0 { b.image } else { SEPARATOR_WIDTH } } else { bw };
            if wrapable && x > 0 && x + w > width {
                row += 1;
                x = 0;
            }
            let y = top + row * bh;
            out.push([x, y, x + w, y + bh]);
            x += w;
            if wrapable && b.state & TBSTATE_WRAP != 0 {
                row += 1;
                x = 0;
            }
        }
        out
    }

    /// The width and height the buttons take together (`TB_GETMAXSIZE`).
    pub fn max_size(&self, width: i32) -> (i32, i32) {
        let rects = self.layout(width);
        let w = rects.iter().map(|r| r[2]).max().unwrap_or(0);
        let h = rects.iter().filter(|r| r[3] > 0).map(|r| r[3] - self.top_margin()).max().unwrap_or(0);
        (w, h)
    }

    /// The number of rows the buttons fall into.
    pub fn row_count(&self, width: i32) -> i32 {
        let rects = self.layout(width);
        let (_, bh) = self.button_size;
        let last = rects.iter().filter(|r| r[3] > 0).map(|r| (r[1] - self.top_margin()) / bh.max(1)).max();
        last.map_or(1, |r| r + 1)
    }

    /// The button at a client point: its index, or a negative number when the point is on a
    /// separator (`-(index + 1)`) or outside every button (`-1`... see `hit_test`).
    pub fn hit_test(&self, width: i32, x: i32, y: i32) -> i32 {
        let rects = self.layout(width);
        for (i, r) in rects.iter().enumerate() {
            if r[2] > r[0] && x >= r[0] && x < r[2] && y >= r[1] && y < r[3] {
                return if self.buttons[i].is_separator() { -(i as i32) - 1 } else { i as i32 };
            }
        }
        -1
    }

    /// `TB_CHECKBUTTON` on a button of a radio group unchecks the others of the group.
    pub fn check(&mut self, index: usize, checked: bool) {
        if checked && self.buttons[index].style & TBSTYLE_CHECKGROUP == TBSTYLE_CHECKGROUP {
            let (mut a, mut b) = (index, index);
            while a > 0 && self.buttons[a - 1].style & TBSTYLE_CHECKGROUP == TBSTYLE_CHECKGROUP {
                a -= 1;
            }
            while b + 1 < self.buttons.len() && self.buttons[b + 1].style & TBSTYLE_CHECKGROUP == TBSTYLE_CHECKGROUP {
                b += 1;
            }
            for k in a..=b {
                if k != index {
                    self.buttons[k].state &= !TBSTATE_CHECKED;
                }
            }
        }
        if checked {
            self.buttons[index].state |= TBSTATE_CHECKED;
        } else {
            self.buttons[index].state &= !TBSTATE_CHECKED;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(image: i32, command: i32, state: u8, style: u8) -> Button {
        Button { image, command, state, style, data: 0, string: 0 }
    }

    fn sample() -> Toolbar {
        let mut t = Toolbar::default();
        for (i, x) in [b(0, 101, 4, 0), b(8, 0, 0, 1), b(1, 102, 4, 2), b(2, 103, 0, 0)].into_iter().enumerate() {
            t.insert(i as i32, x);
        }
        t
    }

    #[test]
    fn the_buttons_are_laid_out_left_to_right_below_a_two_pixel_margin() {
        let t = sample();
        assert_eq!(t.layout(400), [[0, 2, 23, 24], [23, 2, 31, 24], [31, 2, 54, 24], [54, 2, 77, 24]]);
        assert_eq!(t.max_size(400), (77, 22));
    }

    #[test]
    fn a_flat_toolbar_has_no_margin_and_bigger_bitmaps_bigger_buttons() {
        let mut t = sample();
        t.style = TBSTYLE_FLAT;
        t.set_bitmap_size(24, 22);
        assert_eq!(t.button_size, (31, 28));
        assert_eq!(t.layout(400)[0], [0, 0, 31, 28]);
        assert_eq!(t.divider(), 0);
    }

    #[test]
    fn hidden_buttons_have_no_rectangle_and_commands_are_found() {
        let mut t = sample();
        t.buttons[2].state |= TBSTATE_HIDDEN;
        assert_eq!(t.layout(400)[2], [0, 0, 0, 0]);
        assert_eq!(t.layout(400)[3], [31, 2, 54, 24]);
        assert_eq!(t.command_to_index(103), 3);
        assert_eq!(t.command_to_index(999), -1);
        assert_eq!(t.command_to_index(0), 1);
    }

    #[test]
    fn bitmaps_are_counted_in_images() {
        let mut t = Toolbar::default();
        assert_eq!(t.add_bitmap(5, 3), 0);
        assert_eq!(t.add_bitmap(6, 2), 3);
    }

    #[test]
    fn a_radio_group_keeps_one_checked() {
        let mut t = Toolbar::default();
        for i in 0..3 {
            t.insert(i, b(i, 10 + i, 4, TBSTYLE_CHECKGROUP));
        }
        t.check(0, true);
        t.check(2, true);
        assert_eq!(t.buttons.iter().map(|x| x.state & 1).collect::<Vec<_>>(), [0, 0, 1]);
    }
}
