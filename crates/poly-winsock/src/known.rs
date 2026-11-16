//! The sockets a process made and has not closed with `closesocket`, by handle value
//! (specs/windows/ws2_32.dll/WSASocketW.yaml, rule `handle`).
//!
//! Winsock keeps its own table of the sockets it handed out. A program that closes a socket's
//! handle with `CloseHandle` leaves the entry behind: the next `closesocket` of that value still
//! succeeds (0), removes the entry, and a second one fails with `WSAENOTSOCK`.

use alloc::vec::Vec;

#[derive(Debug, Default)]
pub struct Known {
    list: Vec<usize>,
}

impl Known {
    pub const fn new() -> Known {
        Known { list: Vec::new() }
    }

    /// A socket with handle value `s` was made.
    pub fn add(&mut self, s: usize) {
        if !self.list.contains(&s) {
            self.list.push(s);
        }
    }

    /// Forget `s`; whether it was in the table.
    pub fn take(&mut self, s: usize) -> bool {
        match self.list.iter().position(|&k| k == s) {
            Some(i) => {
                self.list.swap_remove(i);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_handle_is_still_ours_once() {
        let mut k = Known::new();
        k.add(0x44);
        k.add(0x44);
        // closesocket after CloseHandle: found once, then it is not a socket.
        assert!(k.take(0x44));
        assert!(!k.take(0x44));
        assert!(!k.take(0));
    }
}
