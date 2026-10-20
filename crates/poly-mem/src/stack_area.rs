//! Where kernel thread stacks live in virtual memory (ADR 0050).

use core::ops::Range;

/// A virtual address range cut into equal slots, one kernel stack each: the
/// stack at the top of its slot, the rest of the slot below it never
/// mapped. Running off the bottom of a stack lands in that gap, the guard,
/// and faults, instead of in whatever memory happens to lie below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackArea {
    base: u64,
    slot_size: u64,
    stack_size: u64,
    slots: usize,
}

impl StackArea {
    /// `slots` slots of `slot_size` bytes from `base`, each holding a stack
    /// of `stack_size` bytes. Panics (at compile time in a `const`) unless
    /// all three are page-aligned and the guard is at least a page.
    pub const fn new(base: u64, slot_size: u64, stack_size: u64, slots: usize) -> StackArea {
        assert!(base.is_multiple_of(crate::PAGE_SIZE), "stack area not page-aligned");
        assert!(slot_size.is_multiple_of(crate::PAGE_SIZE), "stack slot not page-aligned");
        assert!(stack_size.is_multiple_of(crate::PAGE_SIZE) && stack_size > 0, "stack size not whole pages");
        assert!(stack_size + crate::PAGE_SIZE <= slot_size, "a stack slot needs room for a guard page");
        assert!(base.checked_add(slot_size * slots as u64).is_some(), "stack area wraps around");
        StackArea { base, slot_size, stack_size, slots }
    }

    /// Bytes from the start of the first slot to the end of the last.
    pub const fn size(&self) -> u64 {
        self.slot_size * self.slots as u64
    }

    pub const fn stack_size(&self) -> u64 {
        self.stack_size
    }

    /// Bytes of guard below each stack.
    pub const fn guard_size(&self) -> u64 {
        self.slot_size - self.stack_size
    }

    pub const fn slots(&self) -> usize {
        self.slots
    }

    fn slot_start(&self, slot: usize) -> Option<u64> {
        (slot < self.slots).then(|| self.base + slot as u64 * self.slot_size)
    }

    /// Where slot `slot`'s stack is mapped.
    pub fn stack(&self, slot: usize) -> Option<Range<u64>> {
        let end = self.slot_start(slot)? + self.slot_size;
        Some(end - self.stack_size..end)
    }

    /// The never-mapped part of slot `slot`, below its stack.
    pub fn guard(&self, slot: usize) -> Option<Range<u64>> {
        let start = self.slot_start(slot)?;
        Some(start..start + self.guard_size())
    }

    /// The slot `addr` lies in, stack or guard.
    fn slot_of(&self, addr: u64) -> Option<usize> {
        let offset = addr.checked_sub(self.base).filter(|&o| o < self.size())?;
        Some((offset / self.slot_size) as usize)
    }

    /// The slot whose guard holds `addr`: a fault there is an overflow of
    /// that slot's stack.
    pub fn guard_hit(&self, addr: u64) -> Option<usize> {
        self.slot_of(addr).filter(|&s| self.guard(s).is_some_and(|g| g.contains(&addr)))
    }

    /// The slot whose stack holds all of `[addr, addr + len)` (at least
    /// `addr` itself when `len` is 0).
    pub fn stack_holding(&self, addr: u64, len: u64) -> Option<usize> {
        let slot = self.slot_of(addr)?;
        let stack = self.stack(slot)?;
        let end = addr.checked_add(len)?;
        (stack.contains(&addr) && end <= stack.end).then_some(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0xffff_c080_0000_0000;
    const AREA: StackArea = StackArea::new(BASE, 0x8000, 0x4000, 64);

    #[test]
    fn stacks_sit_at_the_top_of_their_slots_above_a_guard() {
        assert_eq!(AREA.size(), 2 << 20);
        assert_eq!(AREA.guard_size(), 0x4000);
        assert_eq!(AREA.stack(0), Some(BASE + 0x4000..BASE + 0x8000));
        assert_eq!(AREA.guard(0), Some(BASE..BASE + 0x4000));
        assert_eq!(AREA.stack(63), Some(BASE + 63 * 0x8000 + 0x4000..BASE + 64 * 0x8000));
        assert_eq!(AREA.stack(64), None);
        assert_eq!(AREA.guard(64), None);
    }

    #[test]
    fn guard_hits_name_the_slot_below_whose_stack_they_are() {
        // Just below slot 5's stack: its guard. Its own stack, and the
        // area's edges, are not guard hits.
        let stack = AREA.stack(5).unwrap();
        assert_eq!(AREA.guard_hit(stack.start - 8), Some(5));
        assert_eq!(AREA.guard_hit(AREA.guard(5).unwrap().start), Some(5));
        assert_eq!(AREA.guard_hit(stack.start), None);
        assert_eq!(AREA.guard_hit(stack.end - 1), None);
        assert_eq!(AREA.guard_hit(BASE - 1), None);
        assert_eq!(AREA.guard_hit(BASE + AREA.size()), None);
        // The guard of slot 0 starts the area.
        assert_eq!(AREA.guard_hit(BASE), Some(0));
    }

    #[test]
    fn ranges_on_a_stack_are_found_only_whole() {
        let stack = AREA.stack(7).unwrap();
        assert_eq!(AREA.stack_holding(stack.start, 0x4000), Some(7));
        assert_eq!(AREA.stack_holding(stack.start + 0x100, 0x200), Some(7));
        assert_eq!(AREA.stack_holding(stack.end - 1, 0), Some(7));
        // Past the top into the next slot's guard, or starting in the guard.
        assert_eq!(AREA.stack_holding(stack.end - 0x10, 0x20), None);
        assert_eq!(AREA.stack_holding(stack.start - 0x10, 0x20), None);
        assert_eq!(AREA.stack_holding(u64::MAX, 2), None);
    }

    #[test]
    #[should_panic(expected = "guard page")]
    fn a_slot_without_room_for_a_guard_is_refused() {
        let _ = StackArea::new(BASE, 0x4000, 0x4000, 4);
    }
}
