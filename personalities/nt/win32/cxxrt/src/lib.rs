//! C++ exception handling for Poly's C runtimes (ADR 0099, ADR 0199): the glue between the
//! running process and the decisions of `poly_cxxeh`, shared by `msvcrt.dll` (which has had it
//! since ADR 0099) and `vcruntime140.dll` (ADR 0199). Each DLL exports the frame handlers under
//! its names, calling [`frame_handler`], exports the assembly `_CxxThrowException` (an
//! `/EXPORT` of this crate's symbol) and names its `terminate()` with [`set_terminate_hook`].
//!
//! - A `throw` raises `0xE06D7363` with the object, its ThrowInfo and the
//!   image base. `_CxxThrowException`'s own frame has an unwind handler: when
//!   an unwind that is not the one into a C++ catch block (an `__except`
//!   took the exception) passes it, the exception object is destroyed
//!   first.
//! - The frame handler, during the search, finds the clause that takes the
//!   exception (or calls `terminate`), makes the catch object, and unwinds
//!   to its frame with `ntdll`'s `RtlUnwindEx` and a
//!   `STATUS_UNWIND_CONSOLIDATE` record whose first parameter is
//!   [`catch_callback`]: after the frames below were unwound, `ntdll` calls
//!   it on the stack below them (they stay in place, with the exception
//!   object in the throwing frame), arranged so that a stack walk from it
//!   continues in the catch's frame. The callback runs the catch funclet
//!   through [`poly_cxx_call_catch`], whose frame handler notices when the
//!   catch block is left by an unwind; at the catch block's end it destroys
//!   the exception object and returns where the function goes on, which
//!   `ntdll` resumes with the catch frame's context.
//! - During an unwind the frame handler runs the frame's unwind actions.
//!   Unwind actions, copy constructors and destructors are called through
//!   [`poly_cxx_call_guarded`], whose frame handler calls `terminate` when
//!   an exception leaves them.
//! - Per thread (ADR 0199): the catch blocks in progress, the exception record and context of
//!   the innermost one (`__current_exception`, `__current_exception_context`), and how many
//!   exceptions are thrown and not yet caught (`std::uncaught_exceptions`, C++17
//!   [uncaught.exceptions]: a throw counts until its handler is entered).

#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};
use poly_cxxeh::catches::Catches;
use poly_cxxeh::consts::{CXX_EXCEPTION, MAGIC_V1, STATUS_UNWIND_CONSOLIDATE};
use poly_cxxeh::frame::{
    FrameKind, Search, frame_kind, parent_frame_offset, search, state_during_catch, unwind_actions,
};
use poly_cxxeh::tables::Func;
use poly_cxxeh::throw::{CatchObject, Exception, ThrowInfo, catch_object};
use poly_cxxeh::{Bad, BadAddress, Memory};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RaiseException(code: u32, flags: u32, count: u32, args: *const usize);
    fn RtlPcToFileHeader(pc: *const u8, base: *mut *mut u8) -> *mut u8;
    fn TlsAlloc() -> u32;
    fn TlsFree(index: u32) -> i32;
    fn TlsGetValue(index: u32) -> *mut u8;
    fn TlsSetValue(index: u32, value: *mut u8) -> i32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RtlUnwindEx(frame: u64, target: u64, record: *mut u8, value: u64, context: *mut u8, history: *mut u8) -> !;
}

/// `EXCEPTION_NONCONTINUABLE`, `EXCEPTION_UNWIND` (every unwinding flag),
/// `EXCEPTION_TARGET_UNWIND` (`winnt.h`).
const EXCEPTION_NONCONTINUABLE: u32 = 0x1;
const EXCEPTION_UNWIND: u32 = 0x66;
const EXCEPTION_TARGET_UNWIND: u32 = 0x20;
/// `ExceptionContinueSearch` (`excpt.h`): what every handler here returns.
const CONTINUE_SEARCH: u32 = 1;
/// `sizeof(EXCEPTION_RECORD)` on x64 (`winnt.h`).
const RECORD_SIZE: usize = 0x98;
/// What a funclet gets in R8 besides RDX = the establisher frame (the
/// spec's `funclet_entry`).
const FUNCLET_R8: u64 = 0x100;
/// The slot of the assembly frames below that holds their argument (from
/// the establisher frame, which is RSP after the prolog).
const FRAME_SLOT: u64 = 0x28;

// ----- this process's memory ----------------------------------------------------------

/// Reads this process's memory: the tables and objects of mapped images
/// and the stack, at addresses the compiler's tables and the dispatcher
/// give.
struct Proc;

impl Memory for Proc {
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress> {
        if addr == 0 {
            return Err(BadAddress(addr));
        }
        // SAFETY: the addresses come from the program's exception tables,
        // the record of a throw and the stack, all mapped while the
        // exception is handled; a bad table faults as it would on Windows.
        unsafe { core::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), buf.len()) };
        Ok(())
    }
}

fn read_u32(addr: u64) -> Result<u32, Bad> {
    let mut b = [0u8; 4];
    Proc.read(addr, &mut b).map_err(|e| Bad::Unreadable(e.0))?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(addr: u64) -> Result<u64, Bad> {
    let mut b = [0u8; 8];
    Proc.read(addr, &mut b).map_err(|e| Bad::Unreadable(e.0))?;
    Ok(u64::from_le_bytes(b))
}

/// The record's code, flags and first parameters (`EXCEPTION_RECORD`:
/// code 0, flags 4, `NumberParameters` 0x18, `ExceptionInformation` 0x20).
///
/// # Safety
/// `record` is a valid `EXCEPTION_RECORD`.
unsafe fn read_exception(record: *const u8) -> Exception {
    // SAFETY: the caller's record.
    unsafe {
        let code = record.cast::<u32>().read_unaligned();
        let flags = record.add(4).cast::<u32>().read_unaligned();
        let n = record.add(0x18).cast::<u32>().read_unaligned().min(15) as usize;
        let mut params = [0u64; 15];
        for (i, p) in params.iter_mut().enumerate().take(n) {
            *p = record.add(0x20 + 8 * i).cast::<u64>().read_unaligned();
        }
        Exception::new(code, flags, &params[..n])
    }
}

/// Complete a rethrow record in place with the exception being handled.
///
/// # Safety
/// `record` is a valid, writable `EXCEPTION_RECORD`.
unsafe fn complete_rethrow(record: *mut u8, exc: &mut Exception) {
    if !exc.is_rethrow() {
        return;
    }
    let current = with_catches(|c| c.current().copied());
    if exc.complete_rethrow(current.as_ref()) {
        for i in 1..4 {
            // SAFETY: the caller's record; parameters 1..3 exist (a C++
            // record has four).
            unsafe { record.add(0x20 + 8 * i).cast::<u64>().write_unaligned(exc.params[i]) };
        }
    }
}

// ----- the per-thread state ----------------------------------------------------------------

/// What a thread keeps: its catch blocks in progress, the record and context of the exception
/// the innermost one handles (null outside a catch block), and its count of exceptions thrown
/// and not yet caught. The three slots are read and written by the program through the
/// pointers `__current_exception`, `__current_exception_context` and `__processing_throw`
/// return, so they keep fixed places.
pub struct Thread {
    catches: Catches,
    /// The record and context of each catch block's exception, by its id in `catches`.
    records: alloc::vec::Vec<(u64, *mut u8, *mut u8)>,
    current_record: *mut u8,
    current_context: *mut u8,
    uncaught: i32,
}

impl Thread {
    const fn new() -> Thread {
        Thread {
            catches: Catches::new(),
            records: alloc::vec::Vec::new(),
            current_record: core::ptr::null_mut(),
            current_context: core::ptr::null_mut(),
            uncaught: 0,
        }
    }
}

/// The state of every thread when no TLS index could be had.
struct Fallback(core::cell::UnsafeCell<Thread>);
// SAFETY: used only when TlsAlloc failed, as the one state of every thread (the best that can
// be done then); never held across a call into the program.
unsafe impl Sync for Fallback {}

static TLS: AtomicU32 = AtomicU32::new(u32::MAX);
static FALLBACK: Fallback = Fallback(core::cell::UnsafeCell::new(Thread::new()));

/// This thread's state (made on its first use; it lives as long as the thread).
fn thread() -> *mut Thread {
    let mut index = TLS.load(Ordering::Acquire);
    if index == u32::MAX {
        // SAFETY: no preconditions.
        let new = unsafe { TlsAlloc() };
        match TLS.compare_exchange(u32::MAX, new, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => index = new,
            Err(won) => {
                if new != u32::MAX {
                    // SAFETY: the index this call allocated.
                    unsafe { TlsFree(new) };
                }
                index = won;
            }
        }
    }
    if index == u32::MAX {
        return FALLBACK.0.get();
    }
    // SAFETY: no preconditions; TlsGetValue clears the last error, which a
    // program may still read after the catch.
    let saved = unsafe { GetLastError() };
    // SAFETY: a valid index.
    let mut p = unsafe { TlsGetValue(index) }.cast::<Thread>();
    if p.is_null() {
        p = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Thread::new()));
        // SAFETY: a valid index.
        unsafe { TlsSetValue(index, p.cast()) };
    }
    // SAFETY: as above.
    unsafe { SetLastError(saved) };
    p
}

/// Run `f` on this thread's list. Never held across a call into the
/// program: the program's code may throw and catch again.
/// Afterwards the current exception is the innermost running catch block's again.
fn with_catches<R>(f: impl FnOnce(&mut Catches) -> R) -> R {
    // SAFETY: this thread's own state; `f` does not call out of this module.
    let t = unsafe { &mut *thread() };
    let r = f(&mut t.catches);
    t.records.retain(|&(id, _, _)| t.catches.get(id).is_some());
    let innermost = t.catches.entries().iter().rev().find(|c| !c.abandoned).map(|c| c.id);
    let (record, context) = innermost
        .and_then(|id| t.records.iter().find(|r| r.0 == id))
        .map_or((core::ptr::null_mut(), core::ptr::null_mut()), |r| (r.1, r.2));
    t.current_record = record;
    t.current_context = context;
    r
}

/// Remember the record and context of catch block `id`'s exception.
fn note_record(id: u64, record: *mut u8, context: *mut u8) {
    // SAFETY: this thread's own state.
    let t = unsafe { &mut *thread() };
    t.records.push((id, record, context));
    t.current_record = record;
    t.current_context = context;
}

/// Where this thread keeps the record of the exception its innermost catch block handles
/// (`__current_exception`).
pub fn current_exception_slot() -> *mut *mut u8 {
    // SAFETY: this thread's state, which stays in place.
    unsafe { &raw mut (*thread()).current_record }
}

/// Where this thread keeps that exception's context (`__current_exception_context`).
pub fn current_context_slot() -> *mut *mut u8 {
    // SAFETY: as above.
    unsafe { &raw mut (*thread()).current_context }
}

/// Where this thread counts the exceptions thrown and not yet caught (`__processing_throw`).
pub fn uncaught_slot() -> *mut i32 {
    // SAFETY: as above.
    unsafe { &raw mut (*thread()).uncaught }
}

/// `std::uncaught_exceptions()` for this thread.
pub fn uncaught_exceptions() -> i32 {
    // SAFETY: as above.
    unsafe { (*thread()).uncaught.max(0) }
}

fn add_uncaught(n: i32) {
    // SAFETY: as above.
    unsafe {
        let t = thread();
        (*t).uncaught = (*t).uncaught.saturating_add(n).max(0);
    }
}

// ----- calling the program ------------------------------------------------------------

unsafe extern "C" {
    /// `funclet(funclet, parent_frame, 0x100)` with `id` in the frame's
    /// slot; the catch funclet's continuation address.
    fn poly_cxx_call_catch(funclet: u64, parent_frame: u64, id: u64) -> u64;
    /// `f(a, b, c)`; an exception leaving it calls `terminate`.
    fn poly_cxx_call_guarded(f: u64, a: u64, b: u64, c: u64) -> u64;
}

/// Call `f(a, b, c)` guarded.
fn guarded(f: u64, a: u64, b: u64, c: u64) {
    // SAFETY: `f` is an unwind funclet, copy function or destructor the
    // program's tables name, called with the arguments its kind takes.
    unsafe { poly_cxx_call_guarded(f, a, b, c) };
}

/// Destroy a C++ exception's object (its ThrowInfo's destructor, if any).
fn destroy(e: &Exception) {
    let Ok(ti) = ThrowInfo::read(&Proc, e.throw_info()) else { return };
    if ti.destructor != 0 && e.object() != 0 {
        guarded(e.image_base().wrapping_add(u64::from(ti.destructor)), e.object(), 0, 0);
    }
}

/// The DLL's `terminate()` (its `set_terminate` handler, then `abort`), named by
/// [`set_terminate_hook`] when the DLL is attached.
static TERMINATE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Name the function the runtime calls for `terminate()` (from the DLL's entry point).
pub fn set_terminate_hook(f: fn() -> !) {
    TERMINATE.store(f as usize, Ordering::Release);
}

/// `terminate()`: the hook, or a fail-fast if the DLL named none.
pub fn terminate() -> ! {
    let h = TERMINATE.load(Ordering::Acquire);
    if h == 0 {
        poly_win32_rt::fail_fast(poly_win32_rt::FAST_FAIL_FATAL_APP_EXIT);
    }
    // SAFETY: a `fn() -> !` stored by `set_terminate_hook`.
    let f: fn() -> ! = unsafe { core::mem::transmute(h) };
    f()
}

// ----- the unwind into a catch block ----------------------------------------------------

/// What the search hands the consolidation callback (through the
/// `STATUS_UNWIND_CONSOLIDATE` record's second parameter) and the unwind
/// handlers: it lives in the searching handler's frame, which stays in
/// place until the catch block ends.
#[repr(C)]
struct CatchInfo {
    /// The frame that holds the try block and its function.
    target_frame: u64,
    function: u64,
    /// Its state while the catch block runs, and where its unwind stops.
    state: i32,
    try_low: i32,
    funclet: u64,
    /// The function's establisher frame (RDX for the funclet).
    parent_frame: u64,
    /// The exception the catch block takes (a rethrow completed).
    exception: Exception,
    /// Its record and context as the dispatcher passed them: they stay in place, in frames
    /// below the catch block, while it runs.
    record: *mut u8,
    context: *mut u8,
}

/// The catch an unwind record leads to, if it is ours.
///
/// # Safety
/// A record whose first parameter is [`catch_callback`] was made by
/// [`frame_handler`] with a live [`CatchInfo`] as the second.
unsafe fn consolidation(exc: &Exception) -> Option<&'static CatchInfo> {
    let ours = catch_callback as unsafe extern "C" fn(*mut u8) -> u64 as usize as u64;
    if exc.code == STATUS_UNWIND_CONSOLIDATE && exc.count >= 2 && exc.params[0] == ours && exc.params[1] != 0 {
        // SAFETY: the caller's contract.
        Some(unsafe { &*(exc.params[1] as *const CatchInfo) })
    } else {
        None
    }
}

/// `ntdll` calls this after the unwind to the catch's frame, on the stack
/// below the unwound frames: run the catch block, then end the exception.
/// Returns where the catch's frame goes on.
unsafe extern "C" fn catch_callback(record: *mut u8) -> u64 {
    // SAFETY: `ntdll` passes the record we unwound with.
    let exc = unsafe { read_exception(record) };
    // SAFETY: it is ours (`ntdll` calls the first parameter).
    let Some(info) = (unsafe { consolidation(&exc) }) else { terminate() };
    let id = with_catches(|c| c.push(info.target_frame, info.function, info.state, info.exception));
    note_record(id, info.record, info.context);
    // The exception is caught once its handler is entered.
    add_uncaught(-1);
    // SAFETY: the catch funclet of the clause, with its frame.
    let next = unsafe { poly_cxx_call_catch(info.funclet, info.parent_frame, id) };
    if let Some(e) = with_catches(|c| c.finish(id)) {
        destroy(&e);
    }
    next
}

/// The frame handler of [`poly_cxx_call_catch`]: during a search it
/// completes a rethrow record; when the catch block is left by an unwind
/// its exception object may be destroyed (`Catches::abandon`).
unsafe extern "C" fn catch_frame_handler(record: *mut u8, frame: u64, _context: *mut u8, _dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's record.
    let mut exc = unsafe { read_exception(record) };
    if exc.flags & EXCEPTION_UNWIND == 0 {
        // SAFETY: as above.
        unsafe { complete_rethrow(record, &mut exc) };
        return CONTINUE_SEARCH;
    }
    let Ok(id) = read_u64(frame + FRAME_SLOT) else { return CONTINUE_SEARCH };
    // SAFETY: an unwind record; ours only if it names our callback.
    let taken_by = unsafe { consolidation(&exc) }.map(|i| i.exception);
    if let Some(e) = with_catches(|c| c.abandon(id, taken_by.as_ref())) {
        destroy(&e);
    }
    CONTINUE_SEARCH
}

/// The frame handler of `_CxxThrowException`: an unwind of its own
/// exception that is not the one into a C++ catch block destroys the
/// object first.
unsafe extern "C" fn throw_frame_handler(record: *mut u8, frame: u64, _context: *mut u8, _dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's record.
    let exc = unsafe { read_exception(record) };
    if exc.flags & EXCEPTION_UNWIND != 0
        && exc.is_cxx()
        && let Ok(object) = read_u64(frame + FRAME_SLOT)
        && object != 0
        && exc.object() == object
    {
        // An `__except` took it: it ends here, not caught by C++.
        add_uncaught(-1);
        destroy(&exc);
    }
    CONTINUE_SEARCH
}

/// The frame handler of [`poly_cxx_call_guarded`]: an exception leaving a
/// destructor, unwind action or copy constructor is `terminate()`.
unsafe extern "C" fn guard_frame_handler(record: *mut u8, _frame: u64, _context: *mut u8, _dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's record.
    let exc = unsafe { read_exception(record) };
    if exc.flags & EXCEPTION_UNWIND == 0 {
        terminate();
    }
    CONTINUE_SEARCH
}

// ----- the frame handlers ---------------------------------------------------------------

/// `DISPATCHER_CONTEXT` (`winnt.h`): ControlPc 0, ImageBase 8,
/// FunctionEntry 0x10, TargetIp 0x20, ContextRecord 0x28, HandlerData 0x38,
/// HistoryTable 0x40.
struct Dispatch {
    control_pc: u64,
    image_base: u64,
    function_entry: u64,
    target_ip: u64,
    context: *mut u8,
    handler_data: u64,
    history: *mut u8,
}

impl Dispatch {
    /// # Safety
    /// `dc` is the dispatcher's `DISPATCHER_CONTEXT`.
    unsafe fn read(dc: *const u8) -> Dispatch {
        // SAFETY: the caller's structure.
        unsafe {
            let q = |o: usize| dc.add(o).cast::<u64>().read_unaligned();
            Dispatch {
                control_pc: q(0),
                image_base: q(8),
                function_entry: q(0x10),
                target_ip: q(0x20),
                context: q(0x28) as *mut u8,
                handler_data: q(0x38),
                history: q(0x40) as *mut u8,
            }
        }
    }

    fn rva(&self, addr: u64) -> u32 {
        u32::try_from(addr.wrapping_sub(self.image_base)).unwrap_or(u32::MAX)
    }
}

/// `__CxxFrameHandler3(record, establisher_frame, context, dispatcher)` and its older names
/// (`__CxxFrameHandler`, `__CxxFrameHandler2`; what depends on the FuncInfo's magic number is
/// read from it): the language-specific handler of functions with C++ try blocks or objects to
/// destroy (and of their funclets). Each DLL exports it under the names it has.
///
/// # Safety
/// Called by the exception dispatcher with valid arguments.
pub unsafe extern "C" fn frame_handler(record: *mut u8, frame: u64, context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's DISPATCHER_CONTEXT and record.
    let (d, exc) = unsafe { (Dispatch::read(dc), read_exception(record)) };
    let r = if exc.flags & EXCEPTION_UNWIND == 0 {
        // SAFETY: as above.
        unsafe { search_frame(record, frame, context, &d, exc) }
    } else {
        // SAFETY: as above.
        unsafe { unwind_frame(frame, &d, &exc) }
    };
    let _ = r;
    CONTINUE_SEARCH
}

/// The function's tables, its start (absolute) and its start's rva.
fn tables(d: &Dispatch) -> Result<(Func<'static>, u64, u32), Bad> {
    let f = Func::read(&Proc, d.image_base, read_u32(d.handler_data)?)?;
    let begin = read_u32(d.function_entry)?;
    Ok((f, d.image_base.wrapping_add(u64::from(begin)), begin))
}

/// The frame's state and kind: the state kept while a catch block of one
/// of its try blocks runs, else its control PC's.
fn locate(f: &Func, d: &Dispatch, frame: u64, function: u64, begin: u32) -> Result<(i32, FrameKind), Bad> {
    let state = match with_catches(|c| c.state_of(frame, function)) {
        Some(s) => s,
        None => f.state_at(d.rva(d.control_pc))?,
    };
    Ok((state, frame_kind(f, state, begin)?))
}

/// The function's establisher frame for a frame of `kind`.
fn parent_of(f: &Func, kind: FrameKind, frame: u64) -> Result<u64, Bad> {
    match parent_frame_offset(f, kind)? {
        Some(off) => read_u64(frame.wrapping_add(u64::from(off))),
        None => Ok(frame),
    }
}

/// The search: complete a rethrow, find the clause, make the catch
/// object, unwind to it (does not return then).
///
/// # Safety
/// The dispatcher's record and dispatcher context.
unsafe fn search_frame(
    record: *mut u8,
    frame: u64,
    context: *mut u8,
    d: &Dispatch,
    mut exc: Exception,
) -> Result<(), Bad> {
    // SAFETY: the caller's record.
    unsafe { complete_rethrow(record, &mut exc) };
    let (f, function, begin) = tables(d)?;
    let (state, kind) = locate(&f, d, frame, function, begin)?;
    let (try_index, handler, matched) = match search(&f, kind, state, &exc)? {
        Search::Continue => return Ok(()),
        Search::Terminate => terminate(),
        Search::Catch { try_index, handler, matched, .. } => (try_index, handler, matched),
    };
    let parent = parent_of(&f, kind, frame)?;
    if exc.is_cxx() {
        match catch_object(&Proc, &handler, &matched, &exc, parent)? {
            CatchObject::None => {}
            CatchObject::Value { at, value } => {
                // SAFETY: the catch object's slot in the function's frame.
                unsafe { (at as *mut u64).write_unaligned(value) };
            }
            CatchObject::Bytes { at, from, size } => {
                // SAFETY: the exception object and the catch object's slot,
                // `size` bytes each (the catchable type's size).
                unsafe { core::ptr::copy_nonoverlapping(from as *const u8, at as *mut u8, size as usize) };
            }
            CatchObject::Copy { at, from, function, virtual_bases } => {
                guarded(function, at, from, u64::from(virtual_bases));
            }
        }
    }
    let tb = f.try_block(try_index)?;
    let info = CatchInfo {
        target_frame: frame,
        function,
        state: state_during_catch(&f, &tb),
        try_low: tb.try_low,
        funclet: d.image_base.wrapping_add(u64::from(handler.handler)),
        parent_frame: parent,
        exception: exc,
        record,
        context,
    };
    let mut unwind = [0u8; RECORD_SIZE];
    unwind[0..4].copy_from_slice(&STATUS_UNWIND_CONSOLIDATE.to_le_bytes());
    unwind[4..8].copy_from_slice(&EXCEPTION_NONCONTINUABLE.to_le_bytes());
    unwind[0x18..0x1C].copy_from_slice(&2u32.to_le_bytes());
    let callback = catch_callback as unsafe extern "C" fn(*mut u8) -> u64 as usize as u64;
    unwind[0x20..0x28].copy_from_slice(&callback.to_le_bytes());
    unwind[0x28..0x30].copy_from_slice(&(&raw const info as u64).to_le_bytes());
    // SAFETY: unwind to this frame (its control PC keeps the stack walk of
    // a later exception in the function); `info` stays in place, as this
    // frame does, until the catch block ends.
    unsafe { RtlUnwindEx(frame, d.control_pc, unwind.as_mut_ptr(), 0, d.context, d.history) }
}

/// The unwind: the frame's unwind actions down to the target state.
///
/// # Safety
/// The dispatcher's dispatcher context.
unsafe fn unwind_frame(frame: u64, d: &Dispatch, exc: &Exception) -> Result<(), Bad> {
    // SAFETY: an unwind record; ours only if it names our callback.
    let ours = unsafe { consolidation(exc) };
    let underlying = ours.map_or(*exc, |i| i.exception);
    let (f, function, begin) = tables(d)?;
    if !underlying.is_cxx() && f.info.synchronous_only() {
        return Ok(());
    }
    let (state, kind) = locate(&f, d, frame, function, begin)?;
    let target = exc.flags & EXCEPTION_TARGET_UNWIND != 0;
    let to = match (target, ours) {
        (false, _) => -1,
        (true, Some(i)) => i.try_low,
        (true, None) => f.state_at(d.rva(d.target_ip))?,
    };
    let actions = unwind_actions(&f, kind, state, to)?;
    let parent = parent_of(&f, kind, frame)?;
    with_catches(|c| c.prune(frame, !target));
    for a in actions {
        let funclet = d.image_base.wrapping_add(u64::from(a));
        guarded(funclet, funclet, parent, FUNCLET_R8);
    }
    Ok(())
}

// ----- the throw ------------------------------------------------------------------------

/// Raise the C++ exception (`_CxxThrowException` calls this from its own
/// frame): code `0xE06D7363`, noncontinuable, four parameters: 0x19930520,
/// the object, the ThrowInfo and the base of the image that holds it (0
/// for a rethrow).
unsafe extern "C" fn cxx_raise(object: *mut u8, throw_info: *const u8) -> ! {
    let mut base = core::ptr::null_mut();
    if !throw_info.is_null() {
        // SAFETY: a lookup without preconditions.
        unsafe { RtlPcToFileHeader(throw_info, &mut base) };
    }
    let args = [MAGIC_V1 as usize, object as usize, throw_info as usize, base as usize];
    // Thrown and not yet caught (a rethrow too: C++17 [except.throw]).
    add_uncaught(1);
    // SAFETY: four arguments.
    unsafe { RaiseException(CXX_EXCEPTION, EXCEPTION_NONCONTINUABLE, 4, args.as_ptr()) };
    // A noncontinuable exception does not come back.
    poly_win32_rt::fail_fast(poly_win32_rt::FAST_FAIL_FATAL_APP_EXIT)
}

// `_CxxThrowException(object, throw_info)` keeps the object in its frame's
// slot (for its unwind handler) and raises. `poly_cxx_call_catch` and
// `poly_cxx_call_guarded` call the program with their frame handlers
// around it; the `nop` after each call keeps the return address out of an
// epilog (Learn: a handler is not called for a frame in its epilog). Every
// frame is 16-byte aligned at its calls (x64 calling convention).
core::arch::global_asm!(
    r#"
    .text
    .globl _CxxThrowException
    .p2align 4
    .seh_proc _CxxThrowException
_CxxThrowException:
    .seh_handler {throw_handler}, @unwind
    sub rsp, 0x38
    .seh_stackalloc 0x38
    .seh_endprologue
    mov qword ptr [rsp + 0x28], rcx
    mov qword ptr [rsp + 0x30], rdx
    call {raise}
    int3
    .seh_endproc

    .globl poly_cxx_call_catch
    .p2align 4
    .seh_proc poly_cxx_call_catch
poly_cxx_call_catch:
    .seh_handler {catch_handler}, @unwind, @except
    sub rsp, 0x38
    .seh_stackalloc 0x38
    .seh_endprologue
    mov qword ptr [rsp + 0x28], r8
    mov rax, rcx
    mov r8d, 0x100
    call rax
    nop
    add rsp, 0x38
    ret
    .seh_endproc

    .globl poly_cxx_call_guarded
    .p2align 4
    .seh_proc poly_cxx_call_guarded
poly_cxx_call_guarded:
    .seh_handler {guard_handler}, @except
    sub rsp, 0x28
    .seh_stackalloc 0x28
    .seh_endprologue
    mov rax, rcx
    mov rcx, rdx
    mov rdx, r8
    mov r8, r9
    call rax
    nop
    add rsp, 0x28
    ret
    .seh_endproc
"#,
    throw_handler = sym throw_frame_handler,
    raise = sym cxx_raise,
    catch_handler = sym catch_frame_handler,
    guard_handler = sym guard_frame_handler,
);

// ----- helpers vcruntime140.dll exports (ADR 0199) ----------------------------------------

/// Destroy the object of the C++ exception `record` describes, if it is one with an object
/// and a destructor (`__DestructExceptionObject`).
///
/// # Safety
/// `record` is null or a valid `EXCEPTION_RECORD`.
pub unsafe fn destruct_exception_object(record: *const u8) {
    if record.is_null() {
        return;
    }
    // SAFETY: the caller's record.
    let exc = unsafe { read_exception(record) };
    if exc.is_cxx() && exc.object() != 0 && exc.throw_info() != 0 {
        destroy(&exc);
    }
}

/// Whether `record` is a C++ exception with no object yet: a `throw;` (parameters 1..3 zero,
/// `_CxxThrowException.yaml`).
///
/// # Safety
/// `record` is null or a valid `EXCEPTION_RECORD`.
pub unsafe fn is_rethrow(record: *const u8) -> bool {
    // SAFETY: the caller's record.
    !record.is_null() && unsafe { read_exception(record) }.is_rethrow()
}

/// Whether the C++ exception `record` describes can be caught as the type whose descriptor
/// (`type_info`) is at `type_info`: one of its catchable types has the same decorated name.
///
/// # Safety
/// `record` is null or a valid `EXCEPTION_RECORD`; `type_info` a type descriptor.
pub unsafe fn is_exception_typeof(type_info: u64, record: *const u8) -> bool {
    if record.is_null() || type_info == 0 {
        return false;
    }
    // SAFETY: the caller's record.
    let exc = unsafe { read_exception(record) };
    if !exc.is_cxx() || exc.throw_info() == 0 {
        return false;
    }
    let Ok(thrown) = poly_cxxeh::throw::Thrown::read(&Proc, &exc) else { return false };
    thrown.types.iter().any(|ct| {
        exc.image_base()
            .checked_add(u64::from(ct.type_descriptor))
            .is_some_and(|td| poly_cxxeh::throw::same_name(&Proc, type_info, td).unwrap_or(false))
    })
}

/// `this` moved to a base class as a pointer-to-member displacement says (`mdisp`, `pdisp`,
/// `vdisp`, the fields a catchable type has: through the virtual base table when `pdisp >=
/// 0`), `__AdjustPointer`.
pub fn adjust_pointer(this: u64, mdisp: i32, pdisp: i32, vdisp: i32) -> u64 {
    let ct =
        poly_cxxeh::throw::CatchableType { properties: 0, type_descriptor: 0, mdisp, pdisp, vdisp, size: 0, copy: 0 };
    poly_cxxeh::throw::adjust(&Proc, this, &ct).unwrap_or(this)
}
