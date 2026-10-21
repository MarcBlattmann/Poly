//! `__C_specific_handler` (ADR 0087): the language-specific handler the C
//! compiler gives functions with `__try` / `__except` / `__finally`, whose
//! handler data is a scope table.
//!
//! Learn ("x64 exception handling") leaves the handler data's format to
//! the handler. The layout is `winnt.h`'s `SCOPE_TABLE_AMD64`: a `Count`,
//! then `Count` records of four image-relative `ULONG`s, `BeginAddress`,
//! `EndAddress`, `HandlerAddress`, `JumpTarget`. What Poly reads into the
//! fields (Poly's reading of the names, to be confirmed by the spec agent
//! against programs built by MSVC on the oracle machine, ADR 0087):
//!
//! - a record covers the code from `BeginAddress` up to (not including)
//!   `EndAddress`; records are in the table's order, innermost first;
//! - a record with a `JumpTarget` is a `__try`/`__except`: `HandlerAddress`
//!   is its filter, called as `LONG filter(EXCEPTION_POINTERS*, frame)`,
//!   unless it is `EXCEPTION_EXECUTE_HANDLER` (1) itself — an address no
//!   code can have (1 is inside the image's headers) standing for a
//!   constant filter; on `EXCEPTION_EXECUTE_HANDLER` the handler unwinds to
//!   `JumpTarget` in the establisher frame (`RtlUnwindEx`, with the
//!   exception code as the return value), on
//!   `EXCEPTION_CONTINUE_EXECUTION` (-1) the exception is dismissed, on
//!   `EXCEPTION_CONTINUE_SEARCH` (0) the next record is tried;
//! - a record without a `JumpTarget` (0) is a `__try`/`__finally`: during
//!   an unwind its `HandlerAddress` is called as `void handler(BOOLEAN
//!   abnormal, frame)` with `abnormal` TRUE, unless the unwind targets this
//!   frame at an address inside the same scope (a jump within it);
//! - `DISPATCHER_CONTEXT.ScopeIndex` is the first record not yet looked at
//!   during an unwind, set past a termination handler before it runs, so a
//!   handler that raises does not run twice.

use crate::consts::{EXCEPTION_TARGET_UNWIND, EXCEPTION_UNWIND};
use crate::dispatch::Disposition;
use crate::{BadAddress, Memory};
use alloc::vec::Vec;

/// One `SCOPE_TABLE_AMD64` record (image-relative addresses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub begin: u32,
    pub end: u32,
    pub handler: u32,
    pub jump_target: u32,
}

impl Scope {
    fn covers(&self, rva: u32) -> bool {
        (self.begin..self.end).contains(&rva)
    }
}

/// Records beyond this make the table malformed (a sanity bound).
const MAX_SCOPES: u32 = 4096;

/// Read the scope table at `addr`.
pub fn read_table<M: Memory + ?Sized>(mem: &M, addr: u64) -> Result<Vec<Scope>, BadAddress> {
    let mut b = [0u8; 4];
    mem.read(addr, &mut b)?;
    let count = u32::from_le_bytes(b).min(MAX_SCOPES);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..u64::from(count) {
        let mut r = [0u8; 16];
        mem.read(addr + 4 + i * 16, &mut r)?;
        let f = |o: usize| u32::from_le_bytes(r[o..o + 4].try_into().expect("4 bytes"));
        out.push(Scope { begin: f(0), end: f(4), handler: f(8), jump_target: f(12) });
    }
    Ok(out)
}

/// `EXCEPTION_EXECUTE_HANDLER`, `EXCEPTION_CONTINUE_SEARCH`,
/// `EXCEPTION_CONTINUE_EXECUTION` (`excpt.h`).
pub const EXCEPTION_EXECUTE_HANDLER: i32 = 1;
pub const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
pub const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;

/// What the handler calls: the program's filters and termination handlers,
/// and the unwind to an `__except` block.
pub trait ScopeCalls {
    /// Call the filter at `image_base + rva`; its result.
    fn filter(&mut self, rva: u32) -> i32;
    /// Call the termination handler at `image_base + rva` (abnormal).
    fn termination(&mut self, rva: u32);
    /// `ScopeIndex` is now `index`.
    fn set_scope_index(&mut self, index: u32);
    /// Unwind to `image_base + jump_target` in the establisher frame; does
    /// not return in a real process (a test records it and returns).
    fn unwind_to(&mut self, jump_target: u32);
}

/// The handler's decision for one call.
///
/// `flags` are the record's `ExceptionFlags`; `pc` is the frame's control
/// address and `target` the unwind's target address (`TargetIp`), both
/// image-relative (`target` only during a target unwind of this frame);
/// `scope_index` is `DISPATCHER_CONTEXT.ScopeIndex`.
pub fn handle<C: ScopeCalls>(
    table: &[Scope],
    flags: u32,
    pc: u32,
    target: Option<u32>,
    scope_index: u32,
    calls: &mut C,
) -> Disposition {
    if flags & EXCEPTION_UNWIND == 0 {
        for s in table {
            if !s.covers(pc) || s.jump_target == 0 {
                continue;
            }
            let verdict = if s.handler == EXCEPTION_EXECUTE_HANDLER as u32 {
                EXCEPTION_EXECUTE_HANDLER
            } else {
                calls.filter(s.handler)
            };
            match verdict {
                EXCEPTION_CONTINUE_SEARCH => continue,
                v if v < 0 => return Disposition::ContinueExecution,
                _ => {
                    calls.unwind_to(s.jump_target);
                    // A real unwind does not come back.
                    return Disposition::ContinueSearch;
                }
            }
        }
        return Disposition::ContinueSearch;
    }
    let target = if flags & EXCEPTION_TARGET_UNWIND != 0 { target } else { None };
    for (i, s) in table.iter().enumerate().skip(scope_index as usize) {
        if !s.covers(pc) {
            continue;
        }
        if let Some(t) = target {
            // The unwind ends in this frame: a scope that also holds the
            // target is left by a jump, not unwound; an `__except` block at
            // the target ends the walk.
            if s.covers(t) {
                break;
            }
            if s.jump_target != 0 && s.jump_target == t {
                break;
            }
        }
        if s.jump_target == 0 {
            calls.set_scope_index(i as u32 + 1);
            calls.termination(s.handler);
        }
    }
    Disposition::ContinueSearch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::EXCEPTION_UNWINDING;
    use alloc::vec;

    #[derive(Default)]
    struct Log {
        filters: Vec<u32>,
        verdicts: Vec<i32>,
        terminations: Vec<u32>,
        unwound: Option<u32>,
        index: u32,
    }

    impl ScopeCalls for Log {
        fn filter(&mut self, rva: u32) -> i32 {
            self.filters.push(rva);
            if self.verdicts.is_empty() { EXCEPTION_CONTINUE_SEARCH } else { self.verdicts.remove(0) }
        }
        fn termination(&mut self, rva: u32) {
            self.terminations.push(rva);
        }
        fn set_scope_index(&mut self, index: u32) {
            self.index = index;
        }
        fn unwind_to(&mut self, jump_target: u32) {
            self.unwound = Some(jump_target);
        }
    }

    fn table() -> Vec<Scope> {
        vec![
            // An inner __try/__finally, an inner __try/__except with a filter,
            // an outer __try/__except(EXCEPTION_EXECUTE_HANDLER).
            Scope { begin: 0x110, end: 0x120, handler: 0x500, jump_target: 0 },
            Scope { begin: 0x100, end: 0x140, handler: 0x600, jump_target: 0x150 },
            Scope { begin: 0x100, end: 0x200, handler: 1, jump_target: 0x210 },
        ]
    }

    #[test]
    fn dispatch_asks_filters_inside_out() {
        let mut log = Log::default();
        assert_eq!(handle(&table(), 0, 0x115, None, 0, &mut log), Disposition::ContinueSearch);
        assert_eq!(log.filters, [0x600], "the __finally is not a filter");
        assert_eq!(log.unwound, Some(0x210), "the constant filter takes it");

        let mut log = Log { verdicts: vec![EXCEPTION_EXECUTE_HANDLER], ..Log::default() };
        handle(&table(), 0, 0x115, None, 0, &mut log);
        assert_eq!(log.unwound, Some(0x150));

        let mut log = Log { verdicts: vec![EXCEPTION_CONTINUE_EXECUTION], ..Log::default() };
        assert_eq!(handle(&table(), 0, 0x115, None, 0, &mut log), Disposition::ContinueExecution);
        assert_eq!(log.unwound, None);

        // Outside every scope.
        let mut log = Log::default();
        assert_eq!(handle(&table(), 0, 0x300, None, 0, &mut log), Disposition::ContinueSearch);
        assert!(log.filters.is_empty() && log.unwound.is_none());
    }

    #[test]
    fn unwinds_run_termination_handlers() {
        let mut log = Log::default();
        handle(&table(), EXCEPTION_UNWINDING, 0x115, None, 0, &mut log);
        assert_eq!(log.terminations, [0x500]);
        assert_eq!(log.index, 1);
        assert!(log.filters.is_empty());
        // Already past it (a nested unwind resumes after it).
        let mut log = Log::default();
        handle(&table(), EXCEPTION_UNWINDING, 0x115, None, 1, &mut log);
        assert!(log.terminations.is_empty());
        // A target unwind to the __except at 0x150 in this frame runs the
        // __finally inside it, and stops there.
        let mut log = Log::default();
        handle(&table(), EXCEPTION_UNWINDING | EXCEPTION_TARGET_UNWIND, 0x115, Some(0x150), 0, &mut log);
        assert_eq!(log.terminations, [0x500]);
        // A target inside the __finally's own scope: a jump, nothing runs.
        let mut log = Log::default();
        handle(&table(), EXCEPTION_UNWINDING | EXCEPTION_TARGET_UNWIND, 0x115, Some(0x118), 0, &mut log);
        assert!(log.terminations.is_empty());
    }

    struct Bytes(Vec<u8>);
    impl Memory for Bytes {
        fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress> {
            let a = addr as usize;
            let src = self.0.get(a..a + buf.len()).ok_or(BadAddress(addr))?;
            buf.copy_from_slice(src);
            Ok(())
        }
    }

    #[test]
    fn reads_the_table() {
        let mut b = vec![2, 0, 0, 0];
        for v in [0x10u32, 0x20, 0x30, 0x40, 0x50, 0x60, 1, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let t = read_table(&Bytes(b.clone()), 0).unwrap();
        assert_eq!(
            t,
            [
                Scope { begin: 0x10, end: 0x20, handler: 0x30, jump_target: 0x40 },
                Scope { begin: 0x50, end: 0x60, handler: 1, jump_target: 0 }
            ]
        );
        assert!(read_table(&Bytes(b[..20].to_vec()), 0).is_err());
    }
}
