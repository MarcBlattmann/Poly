//! Input idle (ADR 0245): `WaitForInputIdle` and the wait of `ShellExecuteEx`'s
//! `SEE_MASK_WAITFORINPUTIDLE` (`specs/windows/user32.dll/WaitForInputIdle.yaml`,
//! `specs/windows/shell32.dll/ShellExecuteExW.WaitForInputIdle.yaml`).
//!
//! A process becomes idle for input the first time one of its threads **blocks** for messages
//! with nothing in its queue: `GetMessage` finding the queue empty, `WaitMessage`, or
//! `MsgWaitForMultipleObjects` waiting for input. A `PeekMessage` that finds nothing does not
//! wait, so it is no idle point for `WaitForInputIdle`; the shell's wait does count it (the end of
//! start-up there is the first look at an empty queue). win32k keeps both facts per process
//! ([`InputIdle`]); they are set once and never cleared while the process lives, which is what
//! makes a later `WaitForInputIdle` return at once ("waits only once").

use alloc::collections::BTreeMap;

/// A thread of the process blocked for messages with an empty queue.
pub const BLOCKED: u8 = 1;
/// A thread of the process looked at its empty queue (a `PeekMessage` that found nothing, or a
/// blocking call).
pub const LOOKED: u8 = 2;

/// What win32k knows of each process's start-up: [`BLOCKED`] and [`LOOKED`] bits by process id.
#[derive(Debug, Default)]
pub struct InputIdle(BTreeMap<u32, u8>);

impl InputIdle {
    /// A thread of `pid` found its queue empty and is about to block (`blocking`) or not.
    pub fn note(&mut self, pid: u32, blocking: bool) {
        *self.0.entry(pid).or_insert(0) |= if blocking { BLOCKED | LOOKED } else { LOOKED };
    }

    /// The bits of process `pid`.
    pub fn state(&self, pid: u32) -> u8 {
        self.0.get(&pid).copied().unwrap_or(0)
    }

    /// Process `pid` has ended (its id may be used again).
    pub fn forget(&mut self, pid: u32) {
        self.0.remove(&pid);
    }
}

/// `IMAGE_SUBSYSTEM_WINDOWS_GUI` (`winnt.h`).
pub const SUBSYSTEM_GUI: u16 = 2;
/// `WAIT_TIMEOUT` and `WAIT_FAILED` (`winbase.h`, `winerror.h`).
pub const WAIT_TIMEOUT: u32 = 0x102;
pub const WAIT_FAILED: u32 = u32::MAX;
/// `INFINITE` (`winbase.h`).
pub const INFINITE: u32 = u32::MAX;
/// `ERROR_INVALID_HANDLE`, `ERROR_NOT_GUI_PROCESS` (`winerror.h`).
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_GUI_PROCESS: u32 = 1471;
/// How often a wait looks again (ms).
pub const POLL_MS: u32 = 10;
/// The longest the shell waits for a GUI program that is never idle (the sheet: 55 to 70 s).
pub const SHELL_WAIT_MS: u32 = 60_000;

/// What the waits need of the system.
pub trait IdleEnv {
    /// The process id of process handle `h` and whether the process still runs; `None` when the
    /// handle is not a process handle with a query right (`PROCESS_QUERY_INFORMATION` or
    /// `PROCESS_QUERY_LIMITED_INFORMATION`).
    fn process(&self, h: u64) -> Option<(u32, bool)>;
    /// The subsystem of the process's executable (`IMAGE_SUBSYSTEM_*`), `None` if it cannot be
    /// told.
    fn subsystem(&self, h: u64) -> Option<u16>;
    /// win32k's [`BLOCKED`] and [`LOOKED`] bits of process `pid`.
    fn idle_state(&self, pid: u32) -> u8;
    /// A millisecond clock.
    fn now_ms(&self) -> u64;
    /// Sleep `ms` milliseconds.
    fn sleep(&self, ms: u32);
}

/// `WaitForInputIdle(process, ms)`: the result, and the last error to set (`None`: left alone).
///
/// The handle must be a process handle with a query right (else `ERROR_INVALID_HANDLE`); a process
/// that has ended fails without an error; a console program (the caller itself among them) fails
/// with `ERROR_NOT_GUI_PROCESS`. Then 0 once the process is [`BLOCKED`] or ends, `WAIT_TIMEOUT`
/// when `ms` runs out (0: at once).
pub fn wait_for_input_idle<E: IdleEnv + ?Sized>(env: &E, h: u64, ms: u32) -> (u32, Option<u32>) {
    let Some((pid, running)) = env.process(h) else { return (WAIT_FAILED, Some(ERROR_INVALID_HANDLE)) };
    if !running {
        return (WAIT_FAILED, None);
    }
    match env.subsystem(h) {
        Some(SUBSYSTEM_GUI) => {}
        Some(_) => return (WAIT_FAILED, Some(ERROR_NOT_GUI_PROCESS)),
        None => return (WAIT_FAILED, Some(ERROR_INVALID_HANDLE)),
    }
    match wait(env, h, pid, BLOCKED, ms) {
        true => (0, None),
        false => (WAIT_TIMEOUT, None),
    }
}

/// `ShellExecuteEx`'s `SEE_MASK_WAITFORINPUTIDLE` on the process it started: a GUI program is
/// waited for until it first looks at its empty queue ([`LOOKED`]: an empty `PeekMessage` counts
/// here), ends, or [`SHELL_WAIT_MS`] have passed; anything else (a console program, a handle it
/// cannot ask) is not waited for. Nothing is reported either way.
pub fn shell_wait<E: IdleEnv + ?Sized>(env: &E, h: u64) {
    let Some((pid, true)) = env.process(h) else { return };
    if env.subsystem(h) == Some(SUBSYSTEM_GUI) {
        wait(env, h, pid, LOOKED | BLOCKED, SHELL_WAIT_MS);
    }
}

/// Wait until process `pid` (handle `h`) has one of the bits `want` or has ended (true), or `ms`
/// ran out (false).
fn wait<E: IdleEnv + ?Sized>(env: &E, h: u64, pid: u32, want: u8, ms: u32) -> bool {
    let deadline = (ms != INFINITE).then(|| env.now_ms().saturating_add(u64::from(ms)));
    loop {
        if env.idle_state(pid) & want != 0 || env.process(h).is_none_or(|(_, running)| !running) {
            return true;
        }
        let left = deadline.map(|d| d.saturating_sub(env.now_ms()));
        if left == Some(0) {
            return false;
        }
        env.sleep(left.map_or(POLL_MS, |l| l.min(u64::from(POLL_MS)) as u32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;

    /// A child on a clock: idle bits from `blocked_at` / `looked_at`, ended at `ends_at` (ms).
    struct Child {
        now: Cell<u64>,
        subsystem: u16,
        blocked_at: Option<u64>,
        looked_at: Option<u64>,
        ends_at: Option<u64>,
        rights: bool,
    }

    impl Child {
        fn gui() -> Child {
            Child { now: Cell::new(0), subsystem: 2, blocked_at: None, looked_at: None, ends_at: None, rights: true }
        }
    }

    impl IdleEnv for Child {
        fn process(&self, h: u64) -> Option<(u32, bool)> {
            (h == 7 && self.rights).then(|| (70, self.ends_at.is_none_or(|e| self.now.get() < e)))
        }
        fn subsystem(&self, _: u64) -> Option<u16> {
            Some(self.subsystem)
        }
        fn idle_state(&self, pid: u32) -> u8 {
            assert_eq!(pid, 70);
            let at = |t: Option<u64>| t.is_some_and(|t| self.now.get() >= t);
            let mut s = 0;
            if at(self.blocked_at) {
                s |= BLOCKED | LOOKED;
            }
            if at(self.looked_at) {
                s |= LOOKED;
            }
            s
        }
        fn now_ms(&self) -> u64 {
            self.now.get()
        }
        fn sleep(&self, ms: u32) {
            self.now.set(self.now.get() + u64::from(ms));
        }
    }

    #[test]
    fn the_flags_are_set_once_per_process_and_go_with_it() {
        let mut i = InputIdle::default();
        i.note(5, false);
        assert_eq!(i.state(5), LOOKED);
        i.note(5, true);
        i.note(5, false);
        assert_eq!(i.state(5), LOOKED | BLOCKED);
        assert_eq!(i.state(6), 0);
        i.forget(5);
        assert_eq!(i.state(5), 0);
    }

    #[test]
    fn a_blocking_call_satisfies_the_wait_and_a_peek_does_not() {
        let c = Child { blocked_at: Some(1500), ..Child::gui() };
        assert_eq!(wait_for_input_idle(&c, 7, 10_000), (0, None));
        assert!((1500..1500 + u64::from(POLL_MS)).contains(&c.now.get()));
        let c = Child { looked_at: Some(1500), ends_at: Some(6000), ..Child::gui() };
        assert_eq!(wait_for_input_idle(&c, 7, INFINITE), (0, None));
        assert!(c.now.get() >= 6000, "a peek is no idle point: it waited until the child ended");
    }

    #[test]
    fn timeouts_and_a_later_call_once_idle() {
        let c = Child { blocked_at: Some(1500), ..Child::gui() };
        assert_eq!(wait_for_input_idle(&c, 7, 0), (WAIT_TIMEOUT, None));
        assert_eq!(c.now.get(), 0);
        assert_eq!(wait_for_input_idle(&c, 7, 100), (WAIT_TIMEOUT, None));
        assert_eq!(c.now.get(), 100);
        assert_eq!(wait_for_input_idle(&c, 7, 10_000), (0, None));
        // Busy again later: the process stays idle-once, so the call returns at once.
        let at = c.now.get();
        assert_eq!(wait_for_input_idle(&c, 7, 0), (0, None));
        assert_eq!(c.now.get(), at);
    }

    #[test]
    fn errors_ended_processes_and_console_programs() {
        let c = Child { rights: false, ..Child::gui() };
        assert_eq!(wait_for_input_idle(&c, 7, 100), (WAIT_FAILED, Some(ERROR_INVALID_HANDLE)));
        assert_eq!(wait_for_input_idle(&Child::gui(), 8, 100), (WAIT_FAILED, Some(ERROR_INVALID_HANDLE)));
        let c = Child { subsystem: 3, ..Child::gui() };
        assert_eq!(wait_for_input_idle(&c, 7, 100), (WAIT_FAILED, Some(ERROR_NOT_GUI_PROCESS)));
        // Ended before the call (idle or not): failed, the error left alone.
        let c = Child { blocked_at: Some(0), ends_at: Some(0), ..Child::gui() };
        assert_eq!(wait_for_input_idle(&c, 7, 5000), (WAIT_FAILED, None));
        // Ends during the wait without being idle: satisfied.
        let c = Child { ends_at: Some(1500), ..Child::gui() };
        assert_eq!(wait_for_input_idle(&c, 7, INFINITE), (0, None));
    }

    #[test]
    fn the_shell_counts_a_peek_skips_console_programs_and_gives_up_after_a_minute() {
        let c = Child { looked_at: Some(1500), ..Child::gui() };
        shell_wait(&c, 7);
        assert!((1500..1500 + u64::from(POLL_MS)).contains(&c.now.get()));
        let c = Child { subsystem: 3, blocked_at: Some(1500), ..Child::gui() };
        shell_wait(&c, 7);
        assert_eq!(c.now.get(), 0);
        let c = Child::gui();
        shell_wait(&c, 7);
        assert_eq!(c.now.get(), u64::from(SHELL_WAIT_MS));
        let c = Child { ends_at: Some(3000), ..Child::gui() };
        shell_wait(&c, 7);
        assert!((3000..3000 + u64::from(POLL_MS)).contains(&c.now.get()));
    }
}
