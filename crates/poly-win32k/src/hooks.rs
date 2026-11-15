//! `SetWindowsHookExW` and its chains for the calling process (Learn: Hooks
//! overview; `winuser.h` for the `WH_*` ids). Hook procedures of this
//! process run on the thread whose message they watch, so the table is one
//! per process and needs no kernel: [`Table`] keeps the hooks and answers
//! which procedures run, in which order, for a message of a thread. `user32`
//! calls them (it owns the stack of what `CallNextHookEx` continues).

use alloc::vec::Vec;

// Hook ids (`winuser.h`).
pub const WH_MSGFILTER: i32 = -1;
pub const WH_JOURNALRECORD: i32 = 0;
pub const WH_JOURNALPLAYBACK: i32 = 1;
pub const WH_KEYBOARD: i32 = 2;
pub const WH_GETMESSAGE: i32 = 3;
pub const WH_CALLWNDPROC: i32 = 4;
pub const WH_CBT: i32 = 5;
pub const WH_SYSMSGFILTER: i32 = 6;
pub const WH_MOUSE: i32 = 7;
pub const WH_DEBUG: i32 = 9;
pub const WH_SHELL: i32 = 10;
pub const WH_FOREGROUNDIDLE: i32 = 11;
pub const WH_CALLWNDPROCRET: i32 = 12;
pub const WH_KEYBOARD_LL: i32 = 13;
pub const WH_MOUSE_LL: i32 = 14;

/// `HC_ACTION` and `HC_NOREMOVE` (`winuser.h`): the `nCode` of a hook call.
pub const HC_ACTION: i32 = 0;
pub const HC_NOREMOVE: i32 = 3;

// Errors (`winerror.h`).
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INVALID_HOOK_HANDLE: u32 = 1404;
pub const ERROR_HOOK_NEEDS_HMOD: u32 = 1428;
pub const ERROR_GLOBAL_ONLY_HOOK: u32 = 1429;
pub const ERROR_INVALID_HOOK_FILTER: u32 = 1426;

/// One installed hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hook {
    pub handle: u64,
    pub id: i32,
    pub proc_: u64,
    /// The thread it watches (0: every thread of the process).
    pub tid: u32,
}

/// The hooks of one process, newest last.
#[derive(Debug, Default)]
pub struct Table {
    next: u64,
    list: Vec<Hook>,
}

/// The first hook handle: `HHOOK`s look like small handle values.
const FIRST_HANDLE: u64 = 0x0001_0053;

impl Table {
    pub const fn new() -> Table {
        Table { next: 0, list: Vec::new() }
    }

    /// `SetWindowsHookExW(id, proc_, module, tid)` for a thread of this
    /// process (`tid` 0: all of them): the handle, or the last error.
    pub fn install(&mut self, id: i32, proc_: u64, module: u64, tid: u32) -> Result<u64, u32> {
        if !(WH_MSGFILTER..=WH_MOUSE_LL).contains(&id) || id == 8 {
            return Err(ERROR_INVALID_HOOK_FILTER);
        }
        if proc_ == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let journal = id == WH_JOURNALRECORD || id == WH_JOURNALPLAYBACK;
        let global_only = journal || id == WH_KEYBOARD_LL || id == WH_MOUSE_LL;
        if global_only && tid != 0 {
            return Err(ERROR_GLOBAL_ONLY_HOOK);
        }
        // A hook for every thread runs in other processes too, so its procedure must be in a DLL.
        if tid == 0 && module == 0 && !global_only {
            return Err(ERROR_HOOK_NEEDS_HMOD);
        }
        let handle = FIRST_HANDLE + 4 * self.next;
        self.next += 1;
        self.list.push(Hook { handle, id, proc_, tid });
        Ok(handle)
    }

    /// `UnhookWindowsHookEx`: whether `handle` was an installed hook.
    pub fn remove(&mut self, handle: u64) -> bool {
        let before = self.list.len();
        self.list.retain(|h| h.handle != handle);
        self.list.len() != before
    }

    /// The procedures that run for a message of thread `tid` through hook `id`: the
    /// thread's own hooks first, then the process-wide ones, the newest of each first.
    pub fn chain(&self, id: i32, tid: u32) -> Vec<u64> {
        let pick = |own: bool| {
            self.list.iter().rev().filter(move |h| h.id == id && (h.tid == tid) == own && (own || h.tid == 0))
        };
        pick(true).chain(pick(false)).map(|h| h.proc_).collect()
    }

    /// Whether any hook of `id` exists at all (the quick check of every message).
    pub fn any(&self, id: i32) -> bool {
        self.list.iter().any(|h| h.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_checks_its_arguments() {
        let mut t = Table::new();
        assert_eq!(t.install(99, 1, 0, 5), Err(ERROR_INVALID_HOOK_FILTER));
        assert_eq!(t.install(WH_CALLWNDPROC, 0, 0, 5), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(t.install(WH_CALLWNDPROC, 1, 0, 0), Err(ERROR_HOOK_NEEDS_HMOD));
        assert_eq!(t.install(WH_JOURNALRECORD, 1, 0, 5), Err(ERROR_GLOBAL_ONLY_HOOK));
        assert!(t.install(WH_CALLWNDPROC, 1, 0, 5).is_ok());
        assert!(t.install(WH_KEYBOARD_LL, 1, 0, 0).is_ok());
    }

    #[test]
    fn chains_run_newest_first_thread_before_process() {
        let mut t = Table::new();
        let a = t.install(WH_GETMESSAGE, 10, 7, 0).unwrap();
        t.install(WH_GETMESSAGE, 11, 0, 5).unwrap();
        t.install(WH_GETMESSAGE, 12, 0, 5).unwrap();
        t.install(WH_GETMESSAGE, 13, 0, 6).unwrap();
        t.install(WH_CALLWNDPROC, 14, 0, 5).unwrap();
        assert_eq!(t.chain(WH_GETMESSAGE, 5), [12, 11, 10]);
        assert_eq!(t.chain(WH_GETMESSAGE, 6), [13, 10]);
        assert!(t.any(WH_CALLWNDPROC) && !t.any(WH_CBT));
        assert!(t.remove(a));
        assert!(!t.remove(a));
        assert_eq!(t.chain(WH_GETMESSAGE, 5), [12, 11]);
    }
}
