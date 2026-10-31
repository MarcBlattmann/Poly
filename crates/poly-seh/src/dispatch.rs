//! The walks over a thread's stack that x64 SEH makes (Learn: "x64
//! exception handling", "Unwind procedure", "Language-specific handler";
//! "RtlUnwindEx"):
//!
//! 1. **Dispatch** ([`dispatch_exception`], `RtlDispatchException`): from
//!    the context of the exception, frame by frame towards the caller, each
//!    function with an exception handler (`UNW_FLAG_EHANDLER`) is asked
//!    about the exception. A handler answers with an
//!    `EXCEPTION_DISPOSITION`: continue execution (the exception context is
//!    resumed, as the handler may have changed it), or continue the search.
//! 2. **Unwind** ([`unwind`], `RtlUnwindEx`): a handler that takes the
//!    exception unwinds the stack to its own (target) frame: every function
//!    with a termination handler (`UNW_FLAG_UHANDLER`) between the
//!    exception and the target is called with `EXCEPTION_UNWINDING` (and
//!    `EXCEPTION_TARGET_UNWIND` for the target itself), and the target's
//!    context resumes at the target address with the return value in RAX.
//!
//! A frame without a table entry is a leaf: its return address is at RSP.
//! Every establisher frame must lie in the stack and be 8-byte aligned, and
//! every frame must be above the one before; otherwise the walk stops
//! (`EXCEPTION_STACK_INVALID`), so corrupt stacks and unwind data end the
//! walk instead of looping. [`walk`] is the same walk without handlers, for
//! backtraces.
//!
//! The handlers are called through [`Handlers`]: on the host a test's
//! closures, in our `ntdll` the language-specific handler routines in user
//! code. Nested exceptions and collided unwinds (handler dispositions 2
//! and 3) are reported as `STATUS_INVALID_DISPOSITION` for now
//! (docs/adr/0068).

use crate::consts::*;
use crate::context::Context;
use crate::pdata::{self, RuntimeFunction};
use crate::record::ExceptionRecord;
use crate::unwind::{Handler, unwind_leaf, virtual_unwind};
use crate::{BadAddress, Memory, UnwindError};
use alloc::boxed::Box;
use alloc::vec::Vec;

/// The most frames one walk visits.
pub const MAX_FRAMES: usize = 1024;

/// The thread's stack: `[low, high)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackLimits {
    pub low: u64,
    pub high: u64,
}

impl StackLimits {
    /// Any address.
    pub const ANY: StackLimits = StackLimits { low: 0, high: u64::MAX };

    pub fn contains(&self, addr: u64) -> bool {
        self.low <= addr && addr < self.high
    }
}

/// A function's table entry as `RtlLookupFunctionEntry` returns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionEntry {
    pub image_base: u64,
    pub function: RuntimeFunction,
    /// The address of the `RUNTIME_FUNCTION` itself.
    pub address: u64,
}

/// The process whose stack is walked: its memory, its function tables and
/// the thread's stack limits.
pub trait Target: Memory {
    /// `RtlLookupFunctionEntry`: the entry covering `pc`, or `None` (a leaf
    /// function, or no image there).
    fn function_entry(&self, pc: u64) -> Result<Option<FunctionEntry>, UnwindError>;

    fn stack_limits(&self) -> StackLimits;
}

/// One image: where it is and where its exception directory is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Module {
    pub base: u64,
    pub size: u64,
    /// The exception directory (absolute address) and its entry count.
    pub table: u64,
    pub count: usize,
}

impl Module {
    /// A module from its base, size and exception directory (RVA and size
    /// in bytes, as the PE optional header gives them).
    pub fn from_directory(base: u64, size: u64, rva: u32, bytes: u32) -> Module {
        Module { base, size, table: base.wrapping_add(u64::from(rva)), count: bytes as usize / RuntimeFunction::SIZE }
    }

    pub fn contains(&self, pc: u64) -> bool {
        pc >= self.base && pc - self.base < self.size
    }
}

/// A [`Target`] made of a memory and a list of images.
#[derive(Debug)]
pub struct Modules<M> {
    pub memory: M,
    pub modules: Vec<Module>,
    pub stack: StackLimits,
}

impl<M: Memory> Memory for Modules<M> {
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress> {
        self.memory.read(addr, buf)
    }
}

impl<M: Memory> Target for Modules<M> {
    fn function_entry(&self, pc: u64) -> Result<Option<FunctionEntry>, UnwindError> {
        let Some(m) = self.modules.iter().find(|m| m.contains(pc)) else { return Ok(None) };
        let Ok(rva) = u32::try_from(pc - m.base) else { return Ok(None) };
        let found = pdata::lookup(&self.memory, m.table, m.count, rva)?;
        Ok(found.map(|(i, function)| FunctionEntry {
            image_base: m.base,
            function,
            address: m.table.wrapping_add((i * RuntimeFunction::SIZE) as u64),
        }))
    }

    fn stack_limits(&self) -> StackLimits {
        self.stack
    }
}

/// `EXCEPTION_DISPOSITION` (`excpt.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    ContinueExecution,
    ContinueSearch,
    NestedException,
    CollidedUnwind,
    /// Any other value a handler returned.
    Other(u32),
}

impl Disposition {
    pub fn from_u32(v: u32) -> Disposition {
        match v {
            EXCEPTION_CONTINUE_EXECUTION_DISPOSITION => Disposition::ContinueExecution,
            EXCEPTION_CONTINUE_SEARCH_DISPOSITION => Disposition::ContinueSearch,
            EXCEPTION_NESTED_EXCEPTION_DISPOSITION => Disposition::NestedException,
            EXCEPTION_COLLIDED_UNWIND_DISPOSITION => Disposition::CollidedUnwind,
            v => Disposition::Other(v),
        }
    }
}

/// What a handler gets besides the record and the context: the
/// `DISPATCHER_CONTEXT` fields, with the frame's own context.
#[derive(Clone, Debug)]
pub struct DispatchInfo {
    pub control_pc: u64,
    pub function: FunctionEntry,
    pub establisher_frame: u64,
    /// The unwind's target address (0 while dispatching).
    pub target_ip: u64,
    /// The context of the frame the handler belongs to.
    pub frame: Context,
    pub handler: Handler,
    pub scope_index: u32,
}

/// Calls language-specific handlers: `handler(record, establisher_frame,
/// context, dispatcher_context)`.
pub trait Handlers {
    fn call(
        &mut self,
        record: &mut ExceptionRecord,
        establisher_frame: u64,
        context: &mut Context,
        dispatch: &mut DispatchInfo,
    ) -> Disposition;
}

/// How a dispatch ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchResult {
    /// A handler said to continue execution: resume the (perhaps changed)
    /// exception context.
    ContinueExecution,
    /// No handler took the exception (or the stack walk had to stop): it
    /// goes on unhandled, second chance.
    NotHandled,
    /// A handler continued an `EXCEPTION_NONCONTINUABLE` exception: raise
    /// `STATUS_NONCONTINUABLE_EXCEPTION`.
    Noncontinuable,
    /// A handler returned a disposition the dispatcher does not take here:
    /// raise `STATUS_INVALID_DISPOSITION`.
    InvalidDisposition,
}

/// One frame of a walk: where it is, and what comes next.
struct Step {
    /// The frame's function, if it has a table entry.
    function: Option<FunctionEntry>,
    handler: Option<Handler>,
    establisher_frame: u64,
    /// The caller's context.
    next: Box<Context>,
}

/// Unwind one frame of `cur`, asking for handlers of `handler_type`.
fn step(target: &dyn Target, cur: &Context, handler_type: u8) -> Result<Step, UnwindError> {
    let pc = cur.rip;
    let mut next = Box::new(cur.clone());
    match target.function_entry(pc)? {
        None => {
            unwind_leaf(&mut next, target)?;
            Ok(Step { function: None, handler: None, establisher_frame: cur.rsp(), next })
        }
        Some(fe) => {
            let u = virtual_unwind(handler_type, fe.image_base, pc, &fe.function, &mut next, target, None)?;
            Ok(Step { function: Some(fe), handler: u.handler, establisher_frame: u.establisher_frame, next })
        }
    }
}

/// The frame is usable: its establisher frame is in the stack and aligned,
/// and its caller's frame is above it.
fn sane(limits: &StackLimits, cur: &Context, s: &Step) -> bool {
    limits.contains(s.establisher_frame) && s.establisher_frame.is_multiple_of(8) && s.next.rsp() > cur.rsp()
}

/// `RtlDispatchException`: look for a handler that takes `record`, raised
/// in context `ctx`. The handlers may change both (a handler that
/// continues execution usually fixes the context up).
pub fn dispatch_exception(
    target: &dyn Target,
    handlers: &mut dyn Handlers,
    record: &mut ExceptionRecord,
    ctx: &mut Context,
) -> DispatchResult {
    let limits = target.stack_limits();
    let mut cur = Box::new(ctx.clone());
    for _ in 0..MAX_FRAMES {
        // A zero return address ends the stack (the thread's first frame).
        if cur.rip == 0 {
            return DispatchResult::NotHandled;
        }
        if !limits.contains(cur.rsp()) {
            record.flags |= EXCEPTION_STACK_INVALID;
            return DispatchResult::NotHandled;
        }
        let s = match step(target, &cur, UNW_FLAG_EHANDLER) {
            Ok(s) if sane(&limits, &cur, &s) => s,
            _ => {
                record.flags |= EXCEPTION_STACK_INVALID;
                return DispatchResult::NotHandled;
            }
        };
        if let (Some(handler), Some(function)) = (s.handler, s.function) {
            let mut info = DispatchInfo {
                control_pc: cur.rip,
                function,
                establisher_frame: s.establisher_frame,
                target_ip: 0,
                frame: (*cur).clone(),
                handler,
                scope_index: 0,
            };
            match handlers.call(record, s.establisher_frame, ctx, &mut info) {
                Disposition::ContinueExecution if record.flags & EXCEPTION_NONCONTINUABLE != 0 => {
                    return DispatchResult::Noncontinuable;
                }
                Disposition::ContinueExecution => return DispatchResult::ContinueExecution,
                Disposition::ContinueSearch => {}
                _ => return DispatchResult::InvalidDisposition,
            }
        }
        cur = s.next;
    }
    DispatchResult::NotHandled
}

/// Why an unwind failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwindFailure {
    /// The walk passed the target frame without reaching it
    /// (`STATUS_INVALID_UNWIND_TARGET`).
    InvalidUnwindTarget,
    /// The stack or its unwind data could not be walked
    /// (`STATUS_BAD_STACK`).
    BadStack,
    /// A termination handler returned something but "continue search"
    /// (`STATUS_INVALID_DISPOSITION`).
    InvalidDisposition,
    /// An exit unwind (no target frame) reached the end of the stack, as it
    /// should.
    EndOfStack,
}

/// A record for an unwind that has none (`STATUS_UNWIND`).
pub fn unwind_record(at: u64) -> ExceptionRecord {
    ExceptionRecord::new(STATUS_UNWIND, 0, at, &[])
}

/// `RtlUnwindEx`: unwind from `ctx` to the frame whose establisher frame is
/// `target_frame` (`None`: an exit unwind of the whole stack), calling the
/// termination handlers on the way, and return the context to resume: the
/// target frame's, at `target_ip`, with `return_value` in RAX.
pub fn unwind(
    target: &dyn Target,
    handlers: &mut dyn Handlers,
    target_frame: Option<u64>,
    target_ip: u64,
    record: &mut ExceptionRecord,
    return_value: u64,
    ctx: &Context,
) -> Result<Context, UnwindFailure> {
    let limits = target.stack_limits();
    let mut cur = Box::new(ctx.clone());
    for _ in 0..MAX_FRAMES {
        if cur.rip == 0 || !limits.contains(cur.rsp()) {
            return Err(if target_frame.is_none() { UnwindFailure::EndOfStack } else { UnwindFailure::BadStack });
        }
        let s = match step(target, &cur, UNW_FLAG_UHANDLER) {
            Ok(s) if sane(&limits, &cur, &s) => s,
            _ => return Err(UnwindFailure::BadStack),
        };
        if let Some(t) = target_frame
            && s.establisher_frame > t
        {
            return Err(UnwindFailure::InvalidUnwindTarget);
        }
        let is_target = target_frame == Some(s.establisher_frame);
        if let (Some(handler), Some(function)) = (s.handler, s.function) {
            record.flags |= EXCEPTION_UNWINDING;
            if target_frame.is_none() {
                record.flags |= EXCEPTION_EXIT_UNWIND;
            }
            if is_target {
                record.flags |= EXCEPTION_TARGET_UNWIND;
            } else {
                record.flags &= !EXCEPTION_TARGET_UNWIND;
            }
            let mut info = DispatchInfo {
                control_pc: cur.rip,
                function,
                establisher_frame: s.establisher_frame,
                target_ip,
                frame: (*cur).clone(),
                handler,
                scope_index: 0,
            };
            if handlers.call(record, s.establisher_frame, &mut cur, &mut info) != Disposition::ContinueSearch {
                return Err(UnwindFailure::InvalidDisposition);
            }
        }
        if is_target {
            cur.rip = target_ip;
            cur.gpr[usize::from(RAX)] = return_value;
            return Ok(*cur);
        }
        cur = s.next;
    }
    Err(UnwindFailure::BadStack)
}

/// One frame of a backtrace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub pc: u64,
    pub sp: u64,
    /// Its function's entry, if it has one (else a leaf).
    pub function: Option<FunctionEntry>,
}

/// Walk the stack from `ctx` for a backtrace: at most `max` frames, ending
/// at a zero return address, the end of the stack, or the first frame that
/// cannot be unwound.
pub fn walk(target: &dyn Target, ctx: &Context, max: usize) -> Vec<Frame> {
    let limits = target.stack_limits();
    let mut out = Vec::new();
    let mut cur = Box::new(ctx.clone());
    while out.len() < max.min(MAX_FRAMES) && cur.rip != 0 && limits.contains(cur.rsp()) {
        let Ok(s) = step(target, &cur, UNW_FLAG_NHANDLER) else {
            out.push(Frame { pc: cur.rip, sp: cur.rsp(), function: None });
            break;
        };
        out.push(Frame { pc: cur.rip, sp: cur.rsp(), function: s.function });
        if s.next.rsp() <= cur.rsp() {
            break;
        }
        cur = s.next;
    }
    out
}
