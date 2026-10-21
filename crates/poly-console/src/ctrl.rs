//! A process's control handlers (Learn: "SetConsoleCtrlHandler",
//! "HandlerRoutine callback function", "CTRL+C and CTRL+BREAK Signals").
//!
//! - Handlers are called last registered first, until one returns TRUE;
//!   if none does, the default handler ends the process
//!   (`ExitProcess`, with `STATUS_CONTROL_C_EXIT` — Poly's choice of exit
//!   code, to be confirmed by the spec agent).
//! - `SetConsoleCtrlHandler(NULL, TRUE)` makes the process ignore Ctrl+C
//!   (no handler runs for `CTRL_C_EVENT`); `(NULL, FALSE)` restores it.
//! - Removing a handler that is not registered fails with
//!   `ERROR_INVALID_PARAMETER` (Poly's choice). A handler registered twice
//!   is called twice and removed one registration at a time.
//!
//! The list holds routine addresses; `kernelbase` calls them.

use crate::consts::CTRL_C_EVENT;
use crate::mode::ERROR_INVALID_PARAMETER;
use alloc::vec::Vec;

#[derive(Clone, Debug, Default)]
pub struct CtrlHandlers {
    handlers: Vec<u64>,
    ignore_ctrl_c: bool,
}

impl CtrlHandlers {
    pub const fn new() -> CtrlHandlers {
        CtrlHandlers { handlers: Vec::new(), ignore_ctrl_c: false }
    }

    /// `SetConsoleCtrlHandler(routine, add)`.
    pub fn set(&mut self, routine: u64, add: bool) -> Result<(), u32> {
        if routine == 0 {
            self.ignore_ctrl_c = add;
            return Ok(());
        }
        if add {
            self.handlers.push(routine);
            return Ok(());
        }
        match self.handlers.iter().rposition(|&h| h == routine) {
            Some(i) => {
                self.handlers.remove(i);
                Ok(())
            }
            None => Err(ERROR_INVALID_PARAMETER),
        }
    }

    /// The handlers to call for `event`, in order; empty when the event is
    /// ignored. `None`: the event is ignored altogether (no default
    /// handler either).
    pub fn plan(&self, event: u32) -> Option<Vec<u64>> {
        if event == CTRL_C_EVENT && self.ignore_ctrl_c {
            return None;
        }
        Some(self.handlers.iter().rev().copied().collect())
    }

    pub fn ignores_ctrl_c(&self) -> bool {
        self.ignore_ctrl_c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::CTRL_BREAK_EVENT;

    #[test]
    fn last_registered_first() {
        let mut h = CtrlHandlers::new();
        h.set(0x10, true).unwrap();
        h.set(0x20, true).unwrap();
        h.set(0x10, true).unwrap();
        assert_eq!(h.plan(CTRL_C_EVENT), Some(alloc::vec![0x10, 0x20, 0x10]));
        h.set(0x10, false).unwrap();
        assert_eq!(h.plan(CTRL_C_EVENT), Some(alloc::vec![0x20, 0x10]));
        assert_eq!(h.set(0x30, false), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn ignoring_ctrl_c() {
        let mut h = CtrlHandlers::new();
        h.set(0x10, true).unwrap();
        h.set(0, true).unwrap();
        assert_eq!(h.plan(CTRL_C_EVENT), None);
        assert_eq!(h.plan(CTRL_BREAK_EVENT), Some(alloc::vec![0x10]));
        h.set(0, false).unwrap();
        assert!(h.plan(CTRL_C_EVENT).is_some());
    }
}
