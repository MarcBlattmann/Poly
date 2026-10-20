//! Frame-pointer stack walking for kernel backtraces.
//!
//! With frame pointers on (`-C force-frame-pointers=yes`), every function
//! starts by pushing the caller's frame pointer and pointing its own frame
//! pointer at that slot. The result is a linked list of frame records on the
//! stack, each two words long:
//!
//! ```text
//! fp + 8: return address into the caller
//! fp + 0: caller's frame pointer (0 at the bottom of the stack)
//! ```
//!
//! This is the layout on x86-64 (`rbp`) and AArch64 (`x29`). The panic path
//! walks it on a stack that may be corrupt, so [`FrameWalker`] checks every
//! frame pointer before it reads through it: it must lie inside one of the
//! known stacks, be aligned, and move strictly towards the stack base. The
//! walk is also bounded in depth, so a cycle cannot hang the panic path.

#![no_std]

use core::ops::Range;

/// Why a walk stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// Reached a zero frame pointer or return address: the normal end.
    Bottom,
    /// A frame pointer pointed outside the known stacks.
    OutsideStack,
    /// A frame pointer was not 8-byte aligned.
    Misaligned,
    /// A frame pointer did not move towards the stack base (a loop or garbage).
    NotIncreasing,
    /// The depth limit was reached before the bottom.
    DepthLimit,
}

impl End {
    pub const fn describe(self) -> &'static str {
        match self {
            End::Bottom => "bottom of stack",
            End::OutsideStack => "frame pointer outside the kernel stacks",
            End::Misaligned => "misaligned frame pointer",
            End::NotIncreasing => "frame pointer not increasing",
            End::DepthLimit => "depth limit reached",
        }
    }
}

/// Iterator over the return addresses on a frame-pointer chain, innermost
/// first. Call [`FrameWalker::end`] afterwards to learn why it stopped.
#[derive(Debug)]
pub struct FrameWalker<R> {
    fp: u64,
    stack: Option<Range<u64>>,
    remaining: usize,
    read: R,
    end: Option<End>,
}

impl<R: FnMut(u64) -> u64> FrameWalker<R> {
    /// Walk the chain starting at frame pointer `fp`, yielding at most
    /// `max_depth` return addresses.
    ///
    /// The walk stays on whichever of `stacks` contains `fp`. `read` loads one
    /// word; it is only ever called with an 8-byte-aligned address `a` where
    /// `a..a + 8` lies inside that stack, so the caller can make it a plain
    /// memory read as long as every range in `stacks` is mapped.
    pub fn new(fp: u64, stacks: &[Range<u64>], max_depth: usize, read: R) -> Self {
        let stack = stacks.iter().find(|s| s.contains(&fp)).cloned();
        FrameWalker { fp, stack, remaining: max_depth, read, end: None }
    }

    /// Why the walk stopped, once the iterator has returned `None`.
    pub fn end(&self) -> Option<End> {
        self.end
    }

    fn check(&self) -> Result<Range<u64>, End> {
        if self.fp == 0 {
            return Err(End::Bottom);
        }
        if self.remaining == 0 {
            return Err(End::DepthLimit);
        }
        if !self.fp.is_multiple_of(8) {
            return Err(End::Misaligned);
        }
        let stack = self.stack.clone().ok_or(End::OutsideStack)?;
        // The whole two-word record must be on the stack.
        match self.fp.checked_add(16) {
            Some(record_end) if self.fp >= stack.start && record_end <= stack.end => Ok(stack),
            _ => Err(End::OutsideStack),
        }
    }
}

impl<R: FnMut(u64) -> u64> Iterator for FrameWalker<R> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        if self.end.is_some() {
            return None;
        }
        if let Err(end) = self.check() {
            self.end = Some(end);
            return None;
        }
        let ret = (self.read)(self.fp + 8);
        let caller_fp = (self.read)(self.fp);
        if ret == 0 {
            self.end = Some(End::Bottom);
            return None;
        }
        // Stacks grow down, so callers' frames sit at higher addresses. The
        // return address read from this frame is still good; stop after it.
        if caller_fp != 0 && caller_fp <= self.fp {
            self.end = Some(End::NotIncreasing);
        }
        self.fp = caller_fp;
        self.remaining -= 1;
        Some(ret)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    const BASE: u64 = 0xffff_8000_0001_0000;

    /// A fake stack of `words` 64-bit slots starting at `BASE`.
    struct Stack {
        words: Vec<u64>,
    }

    impl Stack {
        fn new(words: usize) -> Stack {
            Stack { words: std::vec![0; words] }
        }

        fn range(&self) -> Range<u64> {
            BASE..BASE + self.words.len() as u64 * 8
        }

        /// Place a frame record at word `slot`.
        fn frame(&mut self, slot: usize, caller_fp: u64, ret: u64) -> u64 {
            self.words[slot] = caller_fp;
            self.words[slot + 1] = ret;
            BASE + slot as u64 * 8
        }

        fn walk(&self, fp: u64, max: usize) -> (Vec<u64>, Option<End>) {
            let range = self.range();
            let mut w = FrameWalker::new(fp, std::slice::from_ref(&range), max, |a| {
                assert!(a % 8 == 0 && a >= range.start && a + 8 <= range.end, "read outside stack: {a:#x}");
                self.words[((a - BASE) / 8) as usize]
            });
            let rets = w.by_ref().collect();
            (rets, w.end())
        }
    }

    #[test]
    fn walks_a_normal_chain_to_the_bottom() {
        let mut s = Stack::new(64);
        let outer = s.frame(40, 0, 0x1003);
        let middle = s.frame(20, outer, 0x1002);
        let inner = s.frame(4, middle, 0x1001);
        assert_eq!(s.walk(inner, 32), (std::vec![0x1001, 0x1002, 0x1003], Some(End::Bottom)));
    }

    #[test]
    fn zero_return_address_ends_the_walk() {
        let mut s = Stack::new(16);
        let outer = s.frame(8, 0x1234, 0);
        let inner = s.frame(2, outer, 0x1001);
        assert_eq!(s.walk(inner, 32), (std::vec![0x1001], Some(End::Bottom)));
    }

    #[test]
    fn stops_at_the_depth_limit() {
        let mut s = Stack::new(64);
        let mut fp = 0;
        for slot in (0..10).rev() {
            fp = s.frame(slot * 4, fp, 0x2000 + slot as u64);
        }
        let (rets, end) = s.walk(fp, 3);
        assert_eq!(rets, std::vec![0x2000, 0x2001, 0x2002]);
        assert_eq!(end, Some(End::DepthLimit));
    }

    #[test]
    fn rejects_a_frame_pointer_off_the_stack() {
        let mut s = Stack::new(16);
        let inner = s.frame(2, BASE + 0x10_0000, 0x1001);
        assert_eq!(s.walk(inner, 32), (std::vec![0x1001], Some(End::OutsideStack)));
        assert_eq!(s.walk(0x1000, 32), (std::vec![], Some(End::OutsideStack)));
    }

    #[test]
    fn rejects_a_record_that_straddles_the_stack_top() {
        let s = Stack::new(16);
        let last_word = s.range().end - 8;
        assert_eq!(s.walk(last_word, 32), (std::vec![], Some(End::OutsideStack)));
    }

    #[test]
    fn rejects_misaligned_frame_pointers() {
        let mut s = Stack::new(16);
        let inner = s.frame(2, BASE + 8 * 6 + 3, 0x1001);
        assert_eq!(s.walk(inner, 32), (std::vec![0x1001], Some(End::Misaligned)));
    }

    #[test]
    fn a_loop_in_the_chain_terminates() {
        let mut s = Stack::new(16);
        let a = BASE + 8 * 2;
        let b = s.frame(8, a, 0x1002);
        s.frame(2, b, 0x1001);
        // a -> b -> a: the second hop goes backwards.
        assert_eq!(s.walk(a, 32), (std::vec![0x1001, 0x1002], Some(End::NotIncreasing)));
        let self_loop = s.frame(12, BASE + 8 * 12, 0x1003);
        assert_eq!(s.walk(self_loop, 32), (std::vec![0x1003], Some(End::NotIncreasing)));
    }

    #[test]
    fn picks_the_stack_that_contains_the_start() {
        let other = 0x1000..0x2000;
        let mut s = Stack::new(16);
        let inner = s.frame(2, 0, 0x1001);
        let range = s.range();
        let mut w = FrameWalker::new(inner, &[other, range], 8, |a| s.words[((a - BASE) / 8) as usize]);
        assert_eq!(w.next(), Some(0x1001));
        assert_eq!(w.next(), None);
        assert_eq!(w.end(), Some(End::Bottom));
    }

    #[test]
    fn address_overflow_is_rejected() {
        let top = u64::MAX - 7;
        let everything = 0..u64::MAX;
        let mut w = FrameWalker::new(top, std::slice::from_ref(&everything), 8, |_| panic!("must not read"));
        assert_eq!(w.next(), None);
        assert_eq!(w.end(), Some(End::OutsideStack));
    }
}
