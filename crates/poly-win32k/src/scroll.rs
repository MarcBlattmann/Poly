//! Scroll bars: the numbers of one bar (a window's horizontal or vertical bar, or a scroll bar
//! control) and the rules that keep them consistent (`specs/windows/user32.dll/`
//! `GetScrollInfo.yaml`, `SetScrollInfo.yaml`, `GetScrollPos.yaml`, `SetScrollPos.yaml`,
//! `GetScrollRange.yaml`, `SetScrollRange.yaml`, `ShowScrollBar.yaml`, `EnableScrollBar.yaml`).
//! `user32` keeps one [`Bar`] per bar and decides from [`Bar::idle`] whether a window's bar is
//! shown.

/// `nBar` values (`winuser.h`).
pub const SB_HORZ: i32 = 0;
pub const SB_VERT: i32 = 1;
pub const SB_CTL: i32 = 2;
pub const SB_BOTH: i32 = 3;

/// `SCROLLINFO.fMask` bits (`winuser.h`).
pub const SIF_RANGE: u32 = 0x1;
pub const SIF_PAGE: u32 = 0x2;
pub const SIF_POS: u32 = 0x4;
pub const SIF_DISABLENOSCROLL: u32 = 0x8;
pub const SIF_TRACKPOS: u32 = 0x10;
/// `SIF_RANGE | SIF_PAGE | SIF_POS | SIF_TRACKPOS`.
pub const SIF_ALL: u32 = 0x17;

/// `sizeof(SCROLLINFO)`: seven 32-bit fields in both widths.
pub const SCROLLINFO_SIZE: u32 = 28;

/// `winerror.h`.
pub const ERROR_NO_SCROLLBARS: u32 = 1447;
pub const ERROR_INVALID_SCROLLBAR_RANGE: u32 = 1448;

/// `EnableScrollBar`'s `ESB_DISABLE_BOTH`: both arrows disabled.
pub const ESB_DISABLE_BOTH: u8 = 3;

/// The scroll bar control messages (`winuser.h`).
pub const SBM_SETPOS: u32 = 0x00E0;
pub const SBM_GETPOS: u32 = 0x00E1;
pub const SBM_SETRANGE: u32 = 0x00E2;
pub const SBM_GETRANGE: u32 = 0x00E3;
pub const SBM_ENABLE_ARROWS: u32 = 0x00E4;
pub const SBM_SETRANGEREDRAW: u32 = 0x00E6;
pub const SBM_SETSCROLLINFO: u32 = 0x00E9;
pub const SBM_GETSCROLLINFO: u32 = 0x00EA;

/// One bar's range, page and position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    pub min: i32,
    pub max: i32,
    /// Stored unsigned (`SetScrollInfo.yaml` `page`).
    pub page: u32,
    pub pos: i32,
}

impl Bar {
    /// A window's bar: 0..100, no page, position 0 (`defaults`).
    pub const WINDOW: Bar = Bar { min: 0, max: 100, page: 0, pos: 0 };
    /// A scroll bar control: 0..0.
    pub const CONTROL: Bar = Bar { min: 0, max: 0, page: 0, pos: 0 };

    /// How many positions the range holds (`nMax - nMin + 1`; up to 2^32 - 1).
    fn span(&self) -> i64 {
        i64::from(self.max) - i64::from(self.min) + 1
    }

    /// The largest position: `nMax - max(nPage - 1, 0)`, not below `nMin`.
    pub fn top(&self) -> i32 {
        let t = i64::from(self.max) - (i64::from(self.page) - 1).max(0);
        t.max(i64::from(self.min)) as i32
    }

    /// A page larger than the range becomes the range's size; the position stays inside
    /// `[nMin, top]`.
    fn settle(&mut self) {
        if i64::from(self.page) > self.span() {
            self.page = self.span() as u32;
        }
        self.pos = self.pos.clamp(self.min, self.top());
    }

    /// `SetScrollInfo`: the fields `mask` names, applied together; the position after it.
    /// `nMin` above `nMax` stores `nMin..nMin`.
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

    /// `SetScrollPos`: the position kept inside the range; the previous one.
    pub fn set_pos(&mut self, pos: i32) -> i32 {
        let old = self.pos;
        self.pos = pos;
        self.settle();
        old
    }

    /// `SetScrollRange`: `min` above `max`, or a difference that does not fit a signed 32-bit
    /// number, is `ERROR_INVALID_SCROLLBAR_RANGE` and changes nothing; else the page is kept
    /// and the position moved into the range.
    pub fn set_range(&mut self, min: i32, max: i32) -> Result<(), u32> {
        if min > max || i64::from(max) - i64::from(min) > i64::from(i32::MAX) {
            return Err(ERROR_INVALID_SCROLLBAR_RANGE);
        }
        self.set_info(SIF_RANGE, min, max, 0, 0);
        Ok(())
    }

    /// Nothing to scroll: the range is a single value or the page covers it. A window's bar in
    /// this state is removed (unless `SIF_DISABLENOSCROLL` keeps it).
    pub fn idle(&self) -> bool {
        self.min == self.max || i64::from(self.page) >= self.span()
    }
}

/// `EnableScrollBar` / `SBM_ENABLE_ARROWS`: the arrows' new disabled bits from the low two bits
/// of `arrows` (`ESB_ENABLE_BOTH` clears them, the others add theirs), and whether that changed
/// anything (`EnableScrollBar.yaml` `result`, `extra_bits`).
pub fn enable_arrows(disabled: u8, arrows: u32) -> (u8, bool) {
    let bits = (arrows & 3) as u8;
    let new = if bits == 0 { 0 } else { disabled | bits };
    (new, new != disabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn bar(min: i32, max: i32, page: u32, pos: i32) -> Bar {
        Bar { min, max, page, pos }
    }

    #[test]
    fn positions_are_clamped_to_the_range_less_the_page() {
        let mut b = Bar::WINDOW;
        assert_eq!(b.set_info(SIF_PAGE, 0, 0, 10, 0), 0);
        for (ask, got) in [(50, 50), (95, 91), (91, 91), (92, 91), (1000, 91), (-5, 0), (i32::MAX, 91), (i32::MIN, 0)] {
            assert_eq!(b.set_info(SIF_POS, 0, 0, 0, ask), got, "{ask}");
        }
        assert_eq!(b.set_info(SIF_RANGE | SIF_PAGE | SIF_POS, 10, 60, 20, 35), 35);
        // A smaller range moves the position.
        assert_eq!(b.set_info(SIF_RANGE, 0, 20, 0, 0), 1);
        assert_eq!(b, bar(0, 20, 20, 1));
        assert_eq!(b.set_info(SIF_PAGE, 0, 0, 0, 0), 1);
        // A page larger than the range is the range's size.
        assert_eq!(b.set_info(SIF_PAGE, 0, 0, 500, 0), 0);
        assert_eq!(b, bar(0, 20, 21, 0));
    }

    #[test]
    fn ranges_store_min_above_max_as_one_value_and_huge_ranges() {
        let mut b = bar(0, 100, 10, 40);
        assert_eq!(b.set_info(SIF_RANGE, 50, 10, 0, 0), 50);
        assert_eq!(b, bar(50, 50, 1, 50));
        assert_eq!(b.set_info(SIF_RANGE | SIF_POS, -100, -20, 0, -50), -50);
        assert_eq!(b, bar(-100, -20, 1, -50));
        b.set_info(SIF_RANGE, 0, i32::MAX, 0, 0);
        assert_eq!(b, bar(0, i32::MAX, 1, 0));
        b.set_info(SIF_RANGE, -i32::MAX, i32::MAX, 0, 0);
        assert_eq!(b, bar(-i32::MAX, i32::MAX, 1, 0));
        // Mask 0 and SIF_TRACKPOS change nothing.
        let mut b = bar(0, 100, 10, 40);
        assert_eq!(b.set_info(0, 1, 2, 3, 4), 40);
        assert_eq!(b.set_info(SIF_TRACKPOS, 0, 0, 0, 77), 40);
        assert_eq!(b, bar(0, 100, 10, 40));
    }

    #[test]
    fn a_page_that_covers_the_range_leaves_the_minimum() {
        let mut b = bar(10, 90, 8, 33);
        assert_eq!(b.set_info(SIF_PAGE, 0, 0, 200, 0), 10);
        assert_eq!(b, bar(10, 90, 81, 10));
        assert!(b.idle());
    }

    #[test]
    fn set_pos_returns_the_old_position() {
        let mut b = Bar::WINDOW;
        assert_eq!(b.set_pos(5), 0);
        b.set_range(10, 110).unwrap();
        assert_eq!(b.pos, 10);
        for (ask, old, now) in [(50, 10, 50), (9, 50, 10), (111, 10, 110), (-1000, 110, 10), (i32::MAX, 10, 110)] {
            assert_eq!(b.set_pos(ask), old);
            assert_eq!(b.pos, now);
        }
        // A page of 20 makes 91 the largest position.
        assert_eq!(b.set_info(SIF_PAGE, 0, 0, 20, 0), 91);
        assert_eq!(b.set_pos(95), 91);
        assert_eq!(b.pos, 91);
        // A control's fresh bar holds only 0.
        let mut c = Bar::CONTROL;
        assert_eq!(c.set_pos(70), 0);
        assert_eq!(c.pos, 0);
    }

    #[test]
    fn set_range_refuses_reversed_and_too_wide_ranges() {
        let mut b = Bar::WINDOW;
        b.set_pos(80);
        b.set_range(0, 50).unwrap();
        assert_eq!(b.pos, 50);
        b.set_info(SIF_PAGE, 0, 0, 10, 0);
        b.set_range(0, 200).unwrap();
        assert_eq!(b, bar(0, 200, 10, 41));
        for (min, max) in [(50, 10), (-1, i32::MAX), (i32::MIN, 0), (i32::MIN, i32::MAX)] {
            assert_eq!(b.set_range(min, max), Err(ERROR_INVALID_SCROLLBAR_RANGE));
        }
        assert_eq!(b, bar(0, 200, 10, 41));
        b.set_range(0, i32::MAX).unwrap();
        assert_eq!((b.min, b.max), (0, i32::MAX));
        b.set_range(7, 7).unwrap();
        assert!(b.idle());
    }

    #[test]
    fn idle_bars() {
        assert!(!Bar::WINDOW.idle());
        assert!(Bar::CONTROL.idle());
        assert!(bar(5, 5, 0, 5).idle());
        assert!(!bar(0, 1, 0, 0).idle());
        assert!(bar(0, 100, 101, 0).idle());
        assert!(!bar(0, 100, 100, 0).idle());
    }

    #[test]
    fn arrows_add_up_until_enabled() {
        let mut d = 0;
        let mut seq = Vec::new();
        for a in [3, 3, 0, 0, 1, 2, 3, 4, 0xFFFF] {
            let (n, changed) = enable_arrows(d, a);
            d = n;
            seq.push(changed);
        }
        assert_eq!(seq, [true, false, true, false, true, true, false, true, true]);
    }
}
