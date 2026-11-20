//! The critical policy sections (`userenv.dll` `EnterCriticalPolicySection` /
//! `LeaveCriticalPolicySection`, sheets of the same names). Poly has no Group Policy engine,
//! so a section holds nothing back: what is left is the bookkeeping the sheets observe.
//!
//! - Each enter gives a new value, different from every value still held (16 nested enters
//!   gave 16 different values).
//! - A value is left once, from any thread.
//! - Entering never blocks, whoever holds a section.
//! - The values are not kernel handles: `GetHandleInformation` and `DuplicateHandle` refuse
//!   them, so they are taken from a range no handle table reaches.

use alloc::vec::Vec;

/// `ERROR_INVALID_HANDLE`, `ERROR_INVALID_PARAMETER` (`winerror.h`).
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_PARAMETER: u32 = 87;

/// The first value handed out, and the step between values: aligned like a handle, far above
/// any value a process's handle table gives, and below 2 GiB (it fits a 32-bit process).
const FIRST: usize = 0x5EC0_0004;
const STEP: usize = 4;

/// The sections a process holds.
#[derive(Debug, Default)]
pub struct Sections {
    held: Vec<usize>,
    next: usize,
}

impl Sections {
    pub const fn new() -> Self {
        Self { held: Vec::new(), next: 0 }
    }

    /// `EnterCriticalPolicySection`: a new value.
    pub fn enter(&mut self) -> usize {
        loop {
            let v = FIRST + (self.next % 0x0100_0000) * STEP;
            self.next = self.next.wrapping_add(1);
            if !self.held.contains(&v) {
                self.held.push(v);
                return v;
            }
        }
    }

    /// `LeaveCriticalPolicySection`: `Err(error)` for NULL (87) or a value that is not held (6).
    pub fn leave(&mut self, value: usize) -> Result<(), u32> {
        if value == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        match self.held.iter().position(|&v| v == value) {
            Some(i) => {
                self.held.swap_remove(i);
                Ok(())
            }
            None => Err(ERROR_INVALID_HANDLE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_new_and_left_once() {
        let mut s = Sections::new();
        let a = s.enter();
        let b = s.enter();
        assert_ne!(a, b);
        assert!(a != 0 && a.is_multiple_of(4) && a < 0x8000_0000);
        assert_eq!(s.leave(b), Ok(()));
        assert_eq!(s.leave(a), Ok(()));
        assert_eq!(s.leave(a), Err(ERROR_INVALID_HANDLE));
        assert_eq!(s.leave(0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(s.leave(usize::MAX), Err(ERROR_INVALID_HANDLE));
        assert_eq!(s.leave(0x7FFF0), Err(ERROR_INVALID_HANDLE));
    }

    #[test]
    fn sixteen_nested_enters_are_all_different() {
        let mut s = Sections::new();
        let v: Vec<usize> = (0..16).map(|_| s.enter()).collect();
        for (i, a) in v.iter().enumerate() {
            assert!(!v[i + 1..].contains(a));
        }
        for a in v.iter().rev() {
            assert_eq!(s.leave(*a), Ok(()));
        }
        for _ in 0..300 {
            let x = s.enter();
            assert_eq!(s.leave(x), Ok(()));
        }
    }
}
