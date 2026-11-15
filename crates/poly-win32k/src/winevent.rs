//! WinEvent hooks (`SetWinEventHook`, `UnhookWinEvent`, `NotifyWinEvent`; Learn: "WinEvents" and
//! the reference pages): the hooks of one process, the events raised in it, and the queue of
//! callbacks each installing thread runs when it looks into its message queue (out-of-context
//! hooks only: SetWinEventHook.yaml `delivery`). The state of the table is pure; `user32` calls
//! the procedures.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

// Flags (`winuser.h`).
pub const WINEVENT_OUTOFCONTEXT: u32 = 0;
pub const WINEVENT_SKIPOWNTHREAD: u32 = 1;
pub const WINEVENT_SKIPOWNPROCESS: u32 = 2;
pub const WINEVENT_INCONTEXT: u32 = 4;

// Events (`winuser.h`).
pub const EVENT_OBJECT_CREATE: u32 = 0x8000;
pub const EVENT_OBJECT_DESTROY: u32 = 0x8001;
pub const EVENT_OBJECT_SHOW: u32 = 0x8002;
pub const EVENT_OBJECT_REORDER: u32 = 0x8004;
pub const EVENT_OBJECT_FOCUS: u32 = 0x8005;
pub const EVENT_OBJECT_HIDE: u32 = 0x8003;
pub const EVENT_OBJECT_STATECHANGE: u32 = 0x800A;
pub const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;
pub const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;
pub const EVENT_OBJECT_PARENTCHANGE: u32 = 0x800F;

// Errors (`winerror.h`).
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_FLAGS: u32 = 1004;
pub const ERROR_INVALID_HOOK_FILTER: u32 = 1426;
pub const ERROR_INVALID_FILTER_PROC: u32 = 1427;
pub const ERROR_HOOK_NEEDS_HMOD: u32 = 1428;
pub const ERROR_INVALID_THREAD_ID: u32 = 1444;

/// One installed hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hook {
    pub handle: u64,
    pub min: u32,
    pub max: u32,
    pub proc_: u64,
    /// The process whose events it wants (0: every process).
    pub pid: u32,
    /// The thread whose events it wants (0: every thread).
    pub tid: u32,
    pub flags: u32,
    /// The thread that installed it, which runs its callbacks.
    pub owner: u32,
    /// The installing process.
    pub owner_pid: u32,
}

/// An event waiting for a hook's callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Queued {
    pub hook: u64,
    pub event: u32,
    pub hwnd: u64,
    pub id_object: i32,
    pub id_child: i32,
    pub thread: u32,
    pub time: u32,
}

/// The table of hooks and the queue of events.
#[derive(Debug, Default)]
pub struct Table {
    next: u64,
    hooks: Vec<Hook>,
    queue: VecDeque<Queued>,
}

const FIRST_HANDLE: u64 = 0x0009_0000;

impl Table {
    pub const fn new() -> Table {
        Table { next: 0, hooks: Vec::new(), queue: VecDeque::new() }
    }

    /// `SetWinEventHook`: `Ok(handle)` or the last error. `thread_exists` tells whether
    /// `tid` names a thread; `module` is the module handle the caller gave.
    #[allow(clippy::too_many_arguments)]
    pub fn install(
        &mut self,
        min: u32,
        max: u32,
        module: u64,
        proc_: u64,
        pid: u32,
        tid: u32,
        flags: u32,
        owner: u32,
        owner_pid: u32,
        thread_exists: bool,
    ) -> Result<u64, u32> {
        if min > max {
            return Err(ERROR_INVALID_HOOK_FILTER);
        }
        if proc_ == 0 {
            return Err(ERROR_INVALID_FILTER_PROC);
        }
        if flags & INVALID_FLAGS != 0 {
            return Err(ERROR_INVALID_FLAGS);
        }
        if flags & WINEVENT_INCONTEXT != 0 && module == 0 {
            return Err(ERROR_HOOK_NEEDS_HMOD);
        }
        if tid != 0 && !thread_exists {
            return Err(ERROR_INVALID_THREAD_ID);
        }
        let handle = FIRST_HANDLE + 8 * self.next;
        self.next += 1;
        self.hooks.push(Hook { handle, min, max, proc_, pid, tid, flags, owner, owner_pid });
        Ok(handle)
    }

    /// `UnhookWinEvent` by thread `caller`: only the installing thread removes a hook, and what
    /// was queued for it is not delivered any more.
    pub fn remove(&mut self, handle: u64, caller: u32) -> Result<(), u32> {
        let Some(i) = self.hooks.iter().position(|h| h.handle == handle && h.owner == caller) else {
            return Err(ERROR_INVALID_HANDLE);
        };
        self.hooks.remove(i);
        self.queue.retain(|q| q.hook != handle);
        Ok(())
    }

    /// Whether `handle` is a WinEvent hook (any thread's).
    pub fn is_hook(&self, handle: u64) -> bool {
        self.hooks.iter().any(|h| h.handle == handle)
    }

    /// An event is raised for the thread `event_thread` of process `event_pid`: it is queued for
    /// every hook that wants it, the newest hook first.
    pub fn raise(
        &mut self,
        event: u32,
        hwnd: u64,
        id_object: i32,
        id_child: i32,
        event_thread: u32,
        event_pid: u32,
        time: u32,
    ) {
        let mut matched: Vec<Queued> = Vec::new();
        for h in self.hooks.iter().rev() {
            let wanted = event >= h.min
                && event <= h.max
                && (h.pid == 0 || h.pid == event_pid)
                && (h.tid == 0 || h.tid == event_thread)
                && !(h.flags & WINEVENT_SKIPOWNTHREAD != 0 && event_thread == h.owner)
                && !(h.flags & WINEVENT_SKIPOWNPROCESS != 0 && event_pid == h.owner_pid);
            if wanted {
                matched.push(Queued { hook: h.handle, event, hwnd, id_object, id_child, thread: event_thread, time });
            }
        }
        self.queue.extend(matched);
    }

    /// The next callback thread `tid` has to run: the hook and what to pass it.
    pub fn next_for(&mut self, tid: u32) -> Option<(Hook, Queued)> {
        let i = self.queue.iter().position(|q| self.hooks.iter().any(|h| h.handle == q.hook && h.owner == tid))?;
        let q = self.queue.remove(i)?;
        let hook = *self.hooks.iter().find(|h| h.handle == q.hook)?;
        Some((hook, q))
    }

    /// Whether thread `tid` has a callback to run.
    pub fn pending_for(&self, tid: u32) -> bool {
        self.queue.iter().any(|q| self.hooks.iter().any(|h| h.handle == q.hook && h.owner == tid))
    }

    /// Whether any hook exists (the quick check before an event is worked out).
    pub fn any(&self) -> bool {
        !self.hooks.is_empty()
    }
}

/// The flag bits that make a call fail: the high bit (`0x80000000`) is no flag.
const INVALID_FLAGS: u32 = 0x8000_0000;

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(t: &mut Table, min: u32, max: u32, tid: u32, flags: u32) -> u64 {
        t.install(min, max, 0, 0x100, 0, tid, flags, 5, 1, true).unwrap()
    }

    #[test]
    fn install_checks_what_the_sheet_says() {
        let mut t = Table::new();
        let call = |t: &mut Table, min, max, module, proc_, flags, exists| {
            t.install(min, max, module, proc_, 0, 9, flags, 5, 1, exists)
        };
        assert_eq!(call(&mut t, 5, 1, 0, 1, 0, true), Err(ERROR_INVALID_HOOK_FILTER));
        assert_eq!(call(&mut t, 1, 5, 0, 0, 0, true), Err(ERROR_INVALID_FILTER_PROC));
        assert_eq!(call(&mut t, 1, 5, 0, 1, WINEVENT_INCONTEXT, true), Err(ERROR_HOOK_NEEDS_HMOD));
        assert_eq!(call(&mut t, 1, 5, 7, 1, 0x8000_0000, true), Err(ERROR_INVALID_FLAGS));
        assert_eq!(call(&mut t, 1, 5, 0, 1, 0, false), Err(ERROR_INVALID_THREAD_ID));
        let a = call(&mut t, 1, 5, 0, 1, 8, true).unwrap();
        let b = call(&mut t, 1, 5, 0, 1, 8, true).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn events_go_to_matching_hooks_newest_first_in_order() {
        let mut t = Table::new();
        let a = hook(&mut t, 0xA000, 0xA0FF, 0, 0);
        let c = hook(&mut t, 0xA001, 0xA100, 0, 0);
        let e = hook(&mut t, 0xA000, 0xAFFF, 0, 0);
        t.raise(0xA0FF, 7, -4, 0, 5, 1, 0);
        t.raise(0xA100, 7, -4, 0, 5, 1, 0);
        let mut seen = Vec::new();
        while let Some((h, q)) = t.next_for(5) {
            seen.push((h.handle, q.event));
        }
        assert_eq!(seen, [(e, 0xA0FF), (c, 0xA0FF), (a, 0xA0FF), (e, 0xA100), (c, 0xA100)]);
    }

    #[test]
    fn removal_drops_queued_events_and_only_the_installer_removes() {
        let mut t = Table::new();
        let a = hook(&mut t, 0, 0xFFFF, 0, 0);
        let b = hook(&mut t, 0, 0xFFFF, 0, 0);
        t.raise(1, 7, 0, 0, 5, 1, 0);
        assert_eq!(t.remove(a, 6), Err(ERROR_INVALID_HANDLE));
        assert_eq!(t.remove(a, 5), Ok(()));
        assert_eq!(t.remove(a, 5), Err(ERROR_INVALID_HANDLE));
        assert_eq!(t.next_for(5).map(|(h, _)| h.handle), Some(b));
        assert!(t.next_for(5).is_none());
    }

    #[test]
    fn filters_by_thread_process_and_skip_flags() {
        let mut t = Table::new();
        let own = hook(&mut t, 0, 0xFFFF, 0, WINEVENT_SKIPOWNTHREAD);
        let proc_ = hook(&mut t, 0, 0xFFFF, 0, WINEVENT_SKIPOWNPROCESS);
        let other = hook(&mut t, 0, 0xFFFF, 6, 0);
        t.raise(1, 7, 0, 0, 5, 1, 0);
        let mut got = Vec::new();
        while let Some((h, _)) = t.next_for(5) {
            got.push(h.handle);
        }
        assert!(!got.contains(&own) && !got.contains(&proc_) && !got.contains(&other));
        assert!(got.is_empty());
        t.raise(2, 7, 0, 0, 6, 2, 0);
        let mut got = Vec::new();
        while let Some((h, _)) = t.next_for(5) {
            got.push(h.handle);
        }
        assert_eq!(got, [other, proc_, own]);
    }
}
