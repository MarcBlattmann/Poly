//! Where a translated process's memory goes.
//!
//! The guest's addresses are host addresses: the kernel maps the arm64
//! program at the addresses it was linked for, so a pointer the guest hands
//! to a system call is the kernel's pointer too. Everything the guest can
//! touch lies in one **window** (the translator's [`GuestMem`]), with a flag
//! byte per 16 KiB page saying what the guest may do there; the runtime's
//! own memory (its program, heap, code area and stack) lies outside it, so
//! no guest access reaches it.
//!
//! ```text
//!  4 GiB    program (its preferred address, above the 4 GiB __PAGEZERO)
//!  32 GiB   libraries (poly_dyld::Options::ARM64), one after another
//!  48 GiB   anonymous memory the runtime hands out (mmap; #390 to come)
//!  60 GiB   system area: the start code, thread-local blocks, the TSD page
//!  ~64 GiB  the main thread's stack, growing down from `stack_top`
//!  1 TiB    the runtime's heap, then its code area, then the hand-off block
//! ```
//!
//! [`GuestMem`]: poly_a64_jit::mem::GuestMem

use poly_a64_jit::mem::PAGE_SIZE;

/// The guest's address layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The window: every guest address is in `window_lo..window_end`.
    pub window_lo: u64,
    pub window_end: u64,
    /// Where the libraries start (`poly_dyld::Options::library_base`).
    pub library_base: u64,
    /// Anonymous memory for the guest's `mmap`: `mmap_lo..mmap_end`. Images
    /// must end below `mmap_lo`.
    pub mmap_lo: u64,
    pub mmap_end: u64,
    /// The system area: start code, thread-local blocks, the TSD page.
    pub system: u64,
    pub system_end: u64,
    /// The main thread's stack: `stack_top - stack_size..stack_top`.
    pub stack_top: u64,
    pub stack_size: u64,
}

impl Layout {
    /// Poly's layout: a 60 GiB window from 4 GiB on (see the module docs).
    pub const POLY: Layout = Layout {
        window_lo: 0x1_0000_0000,
        window_end: 0x10_0000_0000,
        library_base: 0x8_0000_0000,
        mmap_lo: 0xC_0000_0000,
        mmap_end: 0xC_8000_0000,
        system: 0xF_0000_0000,
        system_end: 0xF_1000_0000,
        stack_top: 0xF_FF00_0000,
        stack_size: 8 << 20,
    };

    /// A small layout for tests: a window of 256 MiB at `lo` (4 GiB for the
    /// usual program), libraries 64 MiB in.
    pub const fn compact(lo: u64) -> Layout {
        const M: u64 = 1 << 20;
        Layout {
            window_lo: lo,
            window_end: lo + 256 * M,
            library_base: lo + 64 * M,
            mmap_lo: lo + 128 * M,
            mmap_end: lo + 192 * M,
            system: lo + 192 * M,
            system_end: lo + 200 * M,
            stack_top: lo + 256 * M - PAGE_SIZE,
            stack_size: M,
        }
    }

    /// The window's size in bytes.
    pub const fn window_size(&self) -> u64 {
        self.window_end - self.window_lo
    }

    /// Whether the parts are in order, inside the window and 16 KiB aligned.
    pub fn is_valid(&self) -> bool {
        let aligned = [
            self.window_lo,
            self.window_end,
            self.library_base,
            self.mmap_lo,
            self.mmap_end,
            self.system,
            self.system_end,
            self.stack_top,
            self.stack_size,
        ]
        .iter()
        .all(|v| v.is_multiple_of(PAGE_SIZE));
        aligned
            && self.window_lo < self.library_base
            && self.library_base < self.mmap_lo
            && self.mmap_lo < self.mmap_end
            && self.mmap_end <= self.system
            && self.system < self.system_end
            && self.system_end <= self.stack_top - self.stack_size
            && self.stack_top <= self.window_end
    }
}

/// The runtime's heap (outside every guest window): 256 MiB, mapped by
/// the kernel, allocated from by the runtime.
pub const RUNTIME_HEAP: u64 = 0x100_0000_0000;
pub const RUNTIME_HEAP_SIZE: u64 = 256 << 20;
/// The translator's code area: readable, writable and executable.
pub const CODE_AREA: u64 = 0x100_4000_0000;
pub const CODE_AREA_SIZE: u64 = 32 << 20;
/// The hand-off block (at most [`HANDOFF_SIZE`] bytes).
pub const HANDOFF: u64 = 0x100_8000_0000;
pub const HANDOFF_SIZE: u64 = 64 << 10;

/// `x` rounded up to whole 16 KiB guest pages.
pub const fn page_up(x: u64) -> u64 {
    (x + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_are_valid() {
        assert!(Layout::POLY.is_valid());
        assert!(Layout::compact(0x1_0000_0000).is_valid());
        // The runtime's memory is outside Poly's window.
        for a in [RUNTIME_HEAP, CODE_AREA, HANDOFF] {
            assert!(a >= Layout::POLY.window_end);
        }
        const { assert!(RUNTIME_HEAP + RUNTIME_HEAP_SIZE <= CODE_AREA) };
        const { assert!(CODE_AREA + CODE_AREA_SIZE <= HANDOFF) };
        assert_eq!(Layout::POLY.library_base, poly_dyld::Options::ARM64.library_base);
    }
}
