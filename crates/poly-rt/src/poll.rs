//! Waiting for handles: [`poll`] over [`PollEntry`]s.

use crate::{Result, sys};
use core::time::Duration;

pub use crate::abi::events::{ERROR, HANGUP, INVALID, PRIORITY, READABLE, WRITABLE};
pub use crate::abi::{NO_HANDLE, PollEntry};

/// An entry waiting for `events` on `handle`.
pub const fn entry(handle: u32, events: u16) -> PollEntry {
    PollEntry { handle, events, revents: 0 }
}

/// Wait until one of `entries` is ready or `timeout` passed (`None`:
/// forever; zero: only look); fill their `revents`. Returns how many are
/// ready. An interrupted wait (the process is ending) reports none.
pub fn poll(entries: &mut [PollEntry], timeout: Option<Duration>) -> Result<usize> {
    let ns = match timeout {
        None => crate::abi::FOREVER,
        Some(d) => u64::try_from(d.as_nanos()).unwrap_or(crate::abi::FOREVER - 1),
    };
    match sys::poll(entries, ns) {
        Err(crate::Error::Interrupted) => {
            entries.iter_mut().for_each(|e| e.revents = 0);
            Ok(0)
        }
        other => other,
    }
}
