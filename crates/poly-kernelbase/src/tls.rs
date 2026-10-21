//! Thread-local and fiber-local storage indices (`TlsAlloc`, `TlsFree`,
//! `FlsAlloc`, `FlsFree`) and where a TLS slot lives in the TEB.
//!
//! Learn ("Thread Local Storage", `TlsAlloc`): at least
//! `TLS_MINIMUM_AVAILABLE` (64, `winnt.h`) indices, at most 1,088 per
//! process; a new index's slot is zero in every thread; `TlsAlloc` fails
//! with `TLS_OUT_OF_INDEXES` (`winbase.h`). The TEB (`winternl.h`, x64)
//! holds the first 64 slots in `TlsSlots[64]` at TEB + 0x1480 and a
//! pointer to the other 1,024 in `TlsExpansionSlots` at TEB + 0x1780
//! (allocated when a thread first stores into one of them).
//!
//! Fiber-local storage: `FLS_MAXIMUM_AVAILABLE` indices (`winnt.h`); each
//! index may have a callback that runs for a thread's non-null value when
//! the thread ends and when the index is freed (Learn: `FlsAlloc`,
//! `FlsFree`). Poly has no fibers yet, so FLS values are per thread.

use alloc::vec;
use alloc::vec::Vec;

/// `TLS_MINIMUM_AVAILABLE` (`winnt.h`): the slots in the TEB itself.
pub const TLS_MINIMUM_AVAILABLE: u32 = 64;
/// The expansion slots (Learn: 1,088 indices in all).
pub const TLS_EXPANSION_SLOTS: u32 = 1024;
pub const TLS_MAXIMUM: u32 = TLS_MINIMUM_AVAILABLE + TLS_EXPANSION_SLOTS;
/// `TLS_OUT_OF_INDEXES`, `FLS_OUT_OF_INDEXES` (`winbase.h`).
pub const TLS_OUT_OF_INDEXES: u32 = 0xFFFF_FFFF;
pub const FLS_OUT_OF_INDEXES: u32 = 0xFFFF_FFFF;
/// `FLS_MAXIMUM_AVAILABLE` (`winnt.h`).
pub const FLS_MAXIMUM_AVAILABLE: u32 = 4080;

/// `TEB.TlsSlots` and `TEB.TlsExpansionSlots` (`winternl.h`).
pub const TEB_TLS_SLOTS: u64 = 0x1480;
pub const TEB_TLS_EXPANSION_SLOTS: u64 = 0x1780;

/// Where index `index`'s value is for a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// At this offset in the TEB.
    Teb(u64),
    /// At this position of the expansion array.
    Expansion(usize),
}

/// The slot of a TLS index, if it is one.
pub fn slot(index: u32) -> Option<Slot> {
    if index < TLS_MINIMUM_AVAILABLE {
        Some(Slot::Teb(TEB_TLS_SLOTS + 8 * u64::from(index)))
    } else if index < TLS_MAXIMUM {
        Some(Slot::Expansion((index - TLS_MINIMUM_AVAILABLE) as usize))
    } else {
        None
    }
}

/// A set of indices handed out lowest first.
#[derive(Clone, Debug)]
pub struct Indices {
    used: Vec<bool>,
}

impl Indices {
    pub fn new(count: u32) -> Indices {
        Indices { used: vec![false; count as usize] }
    }

    /// The lowest free index, now taken.
    pub fn alloc(&mut self) -> Option<u32> {
        let i = self.used.iter().position(|u| !u)?;
        self.used[i] = true;
        Some(i as u32)
    }

    /// Free `index`: false if it was not allocated (Learn: `TlsFree` /
    /// `FlsFree` fail with `ERROR_INVALID_PARAMETER`).
    pub fn free(&mut self, index: u32) -> bool {
        match self.used.get_mut(index as usize) {
            Some(u) if *u => {
                *u = false;
                true
            }
            _ => false,
        }
    }

    pub fn is_allocated(&self, index: u32) -> bool {
        self.used.get(index as usize).copied().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_follow_the_teb_layout() {
        assert_eq!(slot(0), Some(Slot::Teb(0x1480)));
        assert_eq!(slot(63), Some(Slot::Teb(0x1478 + 0x200)));
        assert_eq!(slot(64), Some(Slot::Expansion(0)));
        assert_eq!(slot(1087), Some(Slot::Expansion(1023)));
        assert_eq!(slot(1088), None);
        // The 64 slots end before the expansion pointer.
        const { assert!(TEB_TLS_SLOTS + 64 * 8 <= TEB_TLS_EXPANSION_SLOTS) };
    }

    #[test]
    fn indices_are_reused_lowest_first() {
        let mut t = Indices::new(TLS_MAXIMUM);
        assert_eq!(t.alloc(), Some(0));
        assert_eq!(t.alloc(), Some(1));
        assert_eq!(t.alloc(), Some(2));
        assert!(t.free(1));
        assert!(!t.free(1));
        assert!(!t.free(5000));
        assert!(!t.is_allocated(1) && t.is_allocated(2));
        assert_eq!(t.alloc(), Some(1));
        let mut n = 3;
        while t.alloc().is_some() {
            n += 1;
        }
        assert_eq!(n, TLS_MAXIMUM);
    }
}
