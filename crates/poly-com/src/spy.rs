//! Initialisation spies (`CoRegisterInitializeSpy`, `CoRevokeInitializeSpy`; Learn:
//! "IInitializeSpy"; sheets of both entries).
//!
//! Spies belong to the thread that registered them; every method of every spy of the thread is
//! called newest registration first (Pre and Post alike). Cookies are 64-bit, never 0, never
//! reused. The table keeps the spy pointers as numbers; `ole32` holds a reference of each.

use alloc::vec::Vec;

/// One registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spy {
    pub cookie: u64,
    pub thread: u32,
    /// The `IInitializeSpy` pointer, as a number.
    pub object: usize,
}

/// The spies of the process.
#[derive(Debug)]
pub struct Spies {
    list: Vec<Spy>,
    next: u64,
}

impl Default for Spies {
    fn default() -> Self {
        Spies::new()
    }
}

impl Spies {
    pub const fn new() -> Spies {
        Spies { list: Vec::new(), next: 1 }
    }

    /// Register `object` for `thread`; the cookie.
    pub fn register(&mut self, thread: u32, object: usize) -> u64 {
        let cookie = self.next;
        self.next += 1;
        self.list.push(Spy { cookie, thread, object });
        cookie
    }

    /// Revoke `cookie` for `thread`: the spy to release, or `None` (`E_INVALIDARG`) for a cookie
    /// that is not a live registration of that thread.
    pub fn revoke(&mut self, thread: u32, cookie: u64) -> Option<usize> {
        let i = self.list.iter().position(|s| s.cookie == cookie && s.thread == thread)?;
        Some(self.list.remove(i).object)
    }

    /// The spies of `thread`, newest registration first.
    pub fn of_thread(&self, thread: u32) -> Vec<usize> {
        self.list.iter().rev().filter(|s| s.thread == thread).map(|s| s.object).collect()
    }

    /// Whether `thread` has any spy (the common case is no: no work then).
    pub fn any_for(&self, thread: u32) -> bool {
        self.list.iter().any(|s| s.thread == thread)
    }

    /// `thread` ended: its spies go; the ones to release.
    pub fn end_thread(&mut self, thread: u32) -> Vec<usize> {
        let mut gone = Vec::new();
        self.list.retain(|s| {
            if s.thread == thread {
                gone.push(s.object);
                false
            } else {
                true
            }
        });
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_per_thread() {
        let mut s = Spies::new();
        let a = s.register(1, 0xA);
        let b = s.register(1, 0xB);
        let c = s.register(1, 0xA);
        s.register(2, 0xC);
        assert!(a != 0 && a != b && b != c);
        assert_eq!(s.of_thread(1), [0xA, 0xB, 0xA]);
        assert_eq!(s.revoke(1, b), Some(0xB));
        assert_eq!(s.of_thread(1), [0xA, 0xA]);
        assert_eq!(s.revoke(1, b), None);
        assert_eq!(s.revoke(1, 0), None);
        assert_eq!(s.revoke(2, a), None, "another thread's cookie");
        assert_eq!(s.end_thread(1), [0xA, 0xA]);
        assert!(!s.any_for(1));
        assert!(s.any_for(2));
    }
}
