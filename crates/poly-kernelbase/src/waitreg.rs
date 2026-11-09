//! Registered waits (Chrome batch 2, ADR 0155): the rules of Learn's
//! `RegisterWaitForSingleObject`, `UnregisterWait` and `UnregisterWaitEx`
//! pages (`threadpoollegacyapiset.h`, `winbase.h`), as host-tested code.
//! The thread that waits and runs the callback is `kernelbase`'s.

use crate::error::*;

/// `WT_EXECUTEDEFAULT` 0, `WT_EXECUTEINIOTHREAD` 1, `WT_EXECUTEINUITHREAD`
/// 2, `WT_EXECUTEINWAITTHREAD` 4, `WT_EXECUTEONLYONCE` 8,
/// `WT_EXECUTELONGFUNCTION` 0x10, `WT_EXECUTEINTIMERTHREAD` 0x20,
/// `WT_EXECUTEINPERSISTENTTHREAD` 0x80, `WT_TRANSFER_IMPERSONATION` 0x100
/// (`winnt.h`).
pub const WT_EXECUTEONLYONCE: u32 = 0x8;
const WT_VALID: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x20 | 0x80 | 0x100;
/// `INFINITE`.
pub const INFINITE: u32 = 0xFFFF_FFFF;

/// A registration's arguments, checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitSpec {
    /// `WT_EXECUTEONLYONCE`: the callback runs once, then the wait ends.
    pub once: bool,
    /// The timeout in milliseconds (`INFINITE`: none); a wait that times
    /// out runs the callback with `TimerOrWaitFired` TRUE and, unless it
    /// is once-only, starts waiting again.
    pub timeout_ms: u32,
}

/// Check `RegisterWaitForSingleObject`'s arguments: a callback is
/// required, the flags must be known ones.
pub fn wait_spec(callback_null: bool, timeout_ms: u32, flags: u32) -> Result<WaitSpec, u32> {
    if callback_null || flags & !WT_VALID != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(WaitSpec { once: flags & WT_EXECUTEONLYONCE != 0, timeout_ms })
}

/// What `UnregisterWaitEx`'s `CompletionEvent` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    /// `NULL`: return at once.
    None,
    /// `INVALID_HANDLE_VALUE`: wait for the callbacks that are running.
    Block,
    /// An event to signal when they are done.
    Event,
}

/// `UnregisterWaitEx`'s `CompletionEvent` argument as a `Completion`.
pub fn completion(handle: isize) -> Completion {
    match handle {
        0 => Completion::None,
        -1 => Completion::Block,
        _ => Completion::Event,
    }
}

/// What `UnregisterWaitEx` does (Learn): the wait is cancelled; if a
/// callback is running, `NULL` and an event make the call fail with
/// `ERROR_IO_PENDING` (the callback finishes later; the event is signaled
/// then), `INVALID_HANDLE_VALUE` waits for it. A call from the callback
/// itself cannot wait for it (that would never end): it returns at once as
/// `NULL` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unregister {
    /// The call's result.
    pub ok: bool,
    /// The error when it fails.
    pub error: u32,
    /// Block until the wait's thread is done.
    pub block: bool,
    /// Signal the event when the callback finishes.
    pub signal_later: bool,
}

pub fn unregister(completion: Completion, callback_running: bool, from_callback: bool) -> Unregister {
    let pending =
        Unregister { ok: false, error: ERROR_IO_PENDING, block: false, signal_later: completion == Completion::Event };
    match (completion, callback_running) {
        (Completion::Block, _) if !from_callback => Unregister { ok: true, error: 0, block: true, signal_later: false },
        (_, true) => pending,
        _ => Unregister { ok: true, error: 0, block: false, signal_later: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_arguments() {
        assert_eq!(wait_spec(false, 100, 0), Ok(WaitSpec { once: false, timeout_ms: 100 }));
        assert_eq!(
            wait_spec(false, INFINITE, WT_EXECUTEONLYONCE | 0x10),
            Ok(WaitSpec { once: true, timeout_ms: INFINITE })
        );
        assert_eq!(wait_spec(true, 0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(wait_spec(false, 0, 0x400), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn unregistering() {
        assert_eq!(completion(0), Completion::None);
        assert_eq!(completion(-1), Completion::Block);
        assert_eq!(completion(0x44), Completion::Event);
        // Nothing running: success at once, whatever the event.
        for c in [Completion::None, Completion::Event] {
            assert_eq!(
                unregister(c, false, false),
                Unregister { ok: true, error: 0, block: false, signal_later: false }
            );
        }
        // A callback running: pending, the event is signaled later.
        assert_eq!(
            unregister(Completion::None, true, false),
            Unregister { ok: false, error: ERROR_IO_PENDING, block: false, signal_later: false }
        );
        assert_eq!(
            unregister(Completion::Event, true, false),
            Unregister { ok: false, error: ERROR_IO_PENDING, block: false, signal_later: true }
        );
        // INVALID_HANDLE_VALUE waits, unless the callback itself asks.
        assert_eq!(
            unregister(Completion::Block, true, false),
            Unregister { ok: true, error: 0, block: true, signal_later: false }
        );
        assert_eq!(
            unregister(Completion::Block, true, true),
            Unregister { ok: false, error: ERROR_IO_PENDING, block: false, signal_later: false }
        );
        assert!(unregister(Completion::Block, false, true).ok);
    }
}
