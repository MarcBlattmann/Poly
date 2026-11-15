//! Scroll bars (spec-win-apps-5: `SetScrollInfo`, `GetScrollInfo`, `SetScrollRange`,
//! `GetScrollRange`, `SetScrollPos`, `GetScrollPos`, `EnableScrollBar`, `ShowScrollBar`).
//!
//! The model of one bar (a window's horizontal or vertical bar, or a scroll bar control) and the
//! rules the sheets `specs/windows/user32.dll/*Scroll*.yaml` fix, as plain data: nothing here
//! knows windows. `user32` keeps a [`Bar`] per window and bar, applies the style bit and the
//! client area changes [`Bar::wants_style`] asks for, and gives the functions their Win32 shapes.
//!
//! Constants are those of `winuser.h`.

/// `SB_HORZ`, `SB_VERT`, `SB_CTL`, `SB_BOTH` (`winuser.h`).
pub const SB_HORZ: i32 = 0;
pub const SB_VERT: i32 = 1;
pub const SB_CTL: i32 = 2;
pub const SB_BOTH: i32 = 3;

/// `SIF_RANGE`, `SIF_PAGE`, `SIF_POS`, `SIF_DISABLENOSCROLL`, `SIF_TRACKPOS`, `SIF_ALL`.
pub const SIF_RANGE: u32 = 0x1;
pub const SIF_PAGE: u32 = 0x2;
pub const SIF_POS: u32 = 0x4;
pub const SIF_DISABLENOSCROLL: u32 = 0x8;
pub const SIF_TRACKPOS: u32 = 0x10;
pub const SIF_ALL: u32 = SIF_RANGE | SIF_PAGE | SIF_POS | SIF_TRACKPOS;

/// `ESB_ENABLE_BOTH`, `ESB_DISABLE_LTUP`, `ESB_DISABLE_RTDN`, `ESB_DISABLE_BOTH`.
pub const ESB_DISABLE_BOTH: u32 = 3;

/// `SBM_*` (`winuser.h`): the messages of a scroll bar control.
pub const SBM_SETPOS: u32 = 0x00E0;
pub const SBM_GETPOS: u32 = 0x00E1;
pub const SBM_SETRANGE: u32 = 0x00E2;
pub const SBM_GETRANGE: u32 = 0x00E3;
pub const SBM_ENABLE_ARROWS: u32 = 0x00E4;
pub const SBM_SETRANGEREDRAW: u32 = 0x00E6;
pub const SBM_SETSCROLLINFO: u32 = 0x00E9;
pub const SBM_GETSCROLLINFO: u32 = 0x00EA;

/// `SCROLLINFO.cbSize` (`winuser.h`).
pub const SCROLLINFO_SIZE: u32 = 28;

/// The state of one scroll bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    pub min: i32,
    pub max: i32,
    /// An unsigned number: 0 is no page.
    pub page: u32,
    pub pos: i32,
    /// The disabled arrows (`ESB_DISABLE_LTUP` 1, `ESB_DISABLE_RTDN` 2).
    pub arrows: u32,
    /// The arrows are disabled because `SIF_DISABLENOSCROLL` kept a bar with nothing to scroll.
    pub no_scroll: bool,
}

impl Bar {
    /// A window's bar when the window has the `WS_HSCROLL` / `WS_VSCROLL` style or the bar is
    /// shown: the range 0..100 (`SetScrollInfo.yaml` `defaults`).
    pub const fn window() -> Bar {
        Bar { min: 0, max: 100, page: 0, pos: 0, arrows: 0, no_scroll: false }
    }

    /// A scroll bar control's bar: the range 0..0.
    pub const fn control() -> Bar {
        Bar { min: 0, max: 0, page: 0, pos: 0, arrows: 0, no_scroll: false }
    }

    /// The number of positions in the range, `max - min + 1`, as a 64-bit number.
    fn size(&self) -> i64 {
        i64::from(self.max) - i64::from(self.min) + 1
    }

    /// The largest position: `max - (page - 1)`, with a page of 0 or 1 meaning `max`.
    pub fn max_pos(&self) -> i32 {
        let back = i64::from(self.page.max(1)) - 1;
        let hi = i64::from(self.max) - back;
        hi.max(i64::from(self.min)) as i32
    }

    /// A position kept inside `[min, max_pos]`.
    pub fn clamp(&self, pos: i32) -> i32 {
        pos.clamp(self.min, self.max_pos())
    }

    /// Whether there is anything to scroll: the page does not cover the range (a page of 0
    /// counts as 1, so a range of one position, 5..5 or 0..0, has nothing to scroll).
    pub fn scrollable(&self) -> bool {
        i64::from(self.page.max(1)) < self.size()
    }

    /// Keep the page inside the range and the position inside the page's reach.
    fn settle(&mut self) {
        let size = self.size().min(i64::from(u32::MAX)) as u32;
        if self.page > size {
            self.page = size;
        }
        self.pos = self.clamp(self.pos);
    }

    /// `SetScrollInfo`'s fields (`mask`: `SIF_RANGE`, `SIF_PAGE`, `SIF_POS`; the other bits mean
    /// nothing here). A range with `min` above `max` becomes `min..min`. Returns the position.
    pub fn set_info(&mut self, mask: u32, min: i32, max: i32, page: u32, pos: i32) -> i32 {
        if mask & SIF_RANGE != 0 {
            self.min = min;
            self.max = max.max(min);
        }
        if mask & SIF_PAGE != 0 {
            self.page = page;
        }
        if mask & SIF_POS != 0 {
            self.pos = pos;
        }
        self.settle();
        self.pos
    }

    /// `SetScrollRange`: `false` (and nothing changed) when `min` is above `max` or the two are
    /// further apart than a signed 32-bit number holds (`SetScrollRange.yaml` `limits`).
    pub fn set_range(&mut self, min: i32, max: i32) -> bool {
        if min > max || i64::from(max) - i64::from(min) > i64::from(i32::MAX) {
            return false;
        }
        self.min = min;
        self.max = max;
        self.settle();
        true
    }

    /// `SetScrollPos`: the position before the call.
    pub fn set_pos(&mut self, pos: i32) -> i32 {
        let old = self.pos;
        self.pos = self.clamp(pos);
        old
    }

    /// `EnableScrollBar` (`wArrows`, only the low two bits count): whether the state changed.
    /// 0 enables both arrows; a disabling flag adds to the arrows already disabled.
    pub fn enable_arrows(&mut self, flags: u32) -> bool {
        let flags = flags & 3;
        let new = if flags == 0 { 0 } else { self.arrows | flags };
        let changed = new != self.arrows;
        self.arrows = new;
        self.no_scroll = false;
        changed
    }

    /// After `SetScrollInfo`/`SetScrollRange`: whether the window's bar keeps its style bit, as
    /// the range and `SIF_DISABLENOSCROLL` (`disable_no_scroll`) ask for. A bar with nothing
    /// to scroll goes, or stays with both arrows disabled; one that has something to scroll again
    /// gets the arrows back that `SIF_DISABLENOSCROLL` had disabled.
    pub fn wants_style(&mut self, disable_no_scroll: bool) -> bool {
        if self.scrollable() {
            if self.no_scroll {
                self.arrows = 0;
                self.no_scroll = false;
            }
            true
        } else if disable_no_scroll {
            self.arrows = ESB_DISABLE_BOTH;
            self.no_scroll = true;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(min: i32, max: i32, page: u32, pos: i32) -> Bar {
        Bar { min, max, page, pos, arrows: 0, no_scroll: false }
    }

    #[test]
    fn positions_stay_inside_the_range_less_the_page() {
        let mut b = Bar::window();
        b.page = 10;
        for (ask, got) in [(95, 91), (1000, 91), (i32::MAX, 91), (-5, 0), (i32::MIN, 0), (50, 50)] {
            assert_eq!(b.set_info(SIF_POS, 0, 0, 0, ask), got);
        }
    }

    #[test]
    fn range_page_and_position_are_applied_together() {
        let mut b = Bar::window();
        assert_eq!(b.set_info(SIF_RANGE | SIF_PAGE | SIF_POS, 10, 60, 20, 35), 35);
        assert_eq!((b.min, b.max, b.page, b.pos), (10, 60, 20, 35));
        // A smaller range moves the position into it (35 to 1) and leaves the page of 20.
        assert_eq!(b.set_info(SIF_RANGE, 0, 20, 0, 0), 1);
        assert_eq!((b.min, b.max, b.page, b.pos), (0, 20, 20, 1));
        // A page larger than the range is the range.
        assert_eq!(b.set_info(SIF_PAGE, 0, 0, 500, 0), 0);
        assert_eq!(b.page, 21);
    }

    #[test]
    fn a_range_with_min_above_max_is_one_position() {
        let mut b = bar(0, 100, 10, 0);
        assert_eq!(b.set_info(SIF_RANGE, 50, 10, 0, 0), 50);
        assert_eq!((b.min, b.max, b.page, b.pos), (50, 50, 1, 50));
        // The page does not come back with a larger range.
        b.set_info(SIF_RANGE, 0, 100, 0, 0);
        assert_eq!(b.page, 1);
    }

    #[test]
    fn nothing_to_scroll_when_the_page_covers_the_range() {
        assert!(!bar(0, 100, 101, 0).scrollable());
        assert!(bar(0, 100, 100, 0).scrollable());
        assert!(bar(0, 100, 0, 0).scrollable());
        assert!(!bar(5, 5, 0, 5).scrollable());
        assert!(!bar(0, 0, 0, 0).scrollable());
        assert!(bar(0, 1, 0, 0).scrollable());
    }

    #[test]
    fn set_range_refuses_what_the_sheet_refuses() {
        let mut b = bar(0, 10, 0, 0);
        assert!(!b.set_range(50, 10));
        assert!(!b.set_range(-1, i32::MAX));
        assert!(!b.set_range(i32::MIN, 0));
        assert!(!b.set_range(i32::MIN, i32::MAX));
        assert_eq!((b.min, b.max), (0, 10));
        assert!(b.set_range(0, i32::MAX));
        assert!(b.set_range(-32767, 32767));
        assert!(b.set_range(7, 7));
    }

    #[test]
    fn set_pos_returns_the_old_position() {
        let mut b = bar(10, 110, 20, 60);
        assert_eq!(b.set_pos(110), 60);
        assert_eq!(b.pos, 91);
        assert_eq!(b.set_pos(-1000), 91);
        assert_eq!(b.pos, 10);
    }

    #[test]
    fn arrows_accumulate_and_zero_enables_both() {
        let mut b = Bar::window();
        assert!(b.enable_arrows(3));
        assert!(!b.enable_arrows(3));
        assert!(b.enable_arrows(0));
        assert!(!b.enable_arrows(0));
        assert!(b.enable_arrows(1));
        assert!(b.enable_arrows(2));
        assert!(!b.enable_arrows(3));
        assert!(b.enable_arrows(4)); // only the low bits count
        assert!(b.enable_arrows(0xFFFF));
    }

    #[test]
    fn disable_no_scroll_keeps_the_bar_with_its_arrows_disabled() {
        let mut b = bar(0, 100, 101, 0);
        assert!(!b.clone().wants_style(false));
        assert!(b.wants_style(true));
        assert_eq!(b.arrows, ESB_DISABLE_BOTH);
        b.page = 10;
        assert!(b.wants_style(false));
        assert_eq!(b.arrows, 0);
    }
}
