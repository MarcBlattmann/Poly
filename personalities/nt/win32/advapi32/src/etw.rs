//! Event Tracing for Windows with no listener (ADR 0155): `EventRegister`
//! and the `EventWrite` family, `RegisterTraceGuidsW`, `TraceEvent` and the
//! `GetTrace*` queries. Poly has no trace sessions, so every provider is
//! disabled: registrations succeed and give handles, writes return
//! `ERROR_SUCCESS` and log nothing, the enable callbacks are never called.
//! The decisions are `poly_etw`'s (host-tested); this file reads the
//! arguments and keeps the process's table.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use poly_etw as etw;

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetLastError(error: u32);
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn TlsAlloc() -> u32;
    fn TlsGetValue(index: u32) -> *mut u8;
    fn TlsSetValue(index: u32, value: *mut u8) -> i32;
}

/// The process's registrations, behind a spin lock (no allocation happens
/// while it is held other than the table's own growth).
struct Table {
    locked: AtomicBool,
    regs: core::cell::UnsafeCell<etw::Registrations>,
}

// SAFETY: access to `regs` is serialised by `locked`.
unsafe impl Sync for Table {}

static TABLE: Table =
    Table { locked: AtomicBool::new(false), regs: core::cell::UnsafeCell::new(etw::Registrations::new()) };

fn with_table<R>(f: impl FnOnce(&mut etw::Registrations) -> R) -> R {
    while TABLE.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let r = f(unsafe { &mut *TABLE.regs.get() });
    TABLE.locked.store(false, Ordering::Release);
    r
}

fn guid_at(p: *const u8) -> [u8; 16] {
    let mut g = [0u8; 16];
    // SAFETY: the caller's GUID of 16 bytes (checked non-null by the callers).
    unsafe { core::ptr::copy_nonoverlapping(p, g.as_mut_ptr(), 16) };
    g
}

// ----- EventRegister and friends (evntprov.h) ----------------------------------------------

/// `EventRegister(ProviderId, EnableCallback, CallbackContext, RegHandle*)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EventRegister(
    provider: *const u8,
    callback: usize,
    context: usize,
    reg_handle: *mut u64,
) -> u32 {
    if provider.is_null() || reg_handle.is_null() {
        return etw::ERROR_INVALID_PARAMETER;
    }
    match with_table(|t| t.register(guid_at(provider), etw::Kind::Modern, callback, context)) {
        Ok(h) => {
            // SAFETY: checked non-null.
            unsafe { reg_handle.write(h) };
            etw::ERROR_SUCCESS
        }
        Err(e) => e,
    }
}

/// `EventUnregister(RegHandle)`.
#[unsafe(no_mangle)]
pub extern "system" fn EventUnregister(reg_handle: u64) -> u32 {
    with_table(|t| t.unregister(reg_handle)).err().unwrap_or(etw::ERROR_SUCCESS)
}

/// `EventSetInformation(RegHandle, InformationClass, EventInformation, InformationLength)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EventSetInformation(reg_handle: u64, class: u32, info: *const u8, length: u32) -> u32 {
    with_table(|t| t.set_information(reg_handle, class, !info.is_null(), length))
}

/// `EventEnabled(RegHandle, EventDescriptor)`: `BOOLEAN`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EventEnabled(reg_handle: u64, _descriptor: *const u8) -> u8 {
    u8::from(with_table(|t| t.enabled(reg_handle)))
}

/// `EventProviderEnabled(RegHandle, Level, Keyword)`: `BOOLEAN`.
#[unsafe(no_mangle)]
pub extern "system" fn EventProviderEnabled(reg_handle: u64, _level: u8, _keyword: u64) -> u8 {
    u8::from(with_table(|t| t.enabled(reg_handle)))
}

/// `EventWrite(RegHandle, EventDescriptor, UserDataCount, UserData)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EventWrite(
    reg_handle: u64,
    _descriptor: *const u8,
    user_data_count: u32,
    user_data: *const u8,
) -> u32 {
    with_table(|t| t.write(reg_handle, user_data_count, !user_data.is_null()))
}

/// `EventWriteTransfer(RegHandle, EventDescriptor, ActivityId, RelatedActivityId, UserDataCount, UserData)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EventWriteTransfer(
    reg_handle: u64,
    _descriptor: *const u8,
    _activity: *const u8,
    _related: *const u8,
    user_data_count: u32,
    user_data: *const u8,
) -> u32 {
    with_table(|t| t.write(reg_handle, user_data_count, !user_data.is_null()))
}

/// `EventWriteEx(RegHandle, EventDescriptor, Filter, Flags, ActivityId, RelatedActivityId, UserDataCount, UserData)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EventWriteEx(
    reg_handle: u64,
    _descriptor: *const u8,
    _filter: u64,
    _flags: u32,
    _activity: *const u8,
    _related: *const u8,
    user_data_count: u32,
    user_data: *const u8,
) -> u32 {
    with_table(|t| t.write(reg_handle, user_data_count, !user_data.is_null()))
}

/// `EventWriteString(RegHandle, Level, Keyword, String)`: a registered handle and a string.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EventWriteString(reg_handle: u64, _level: u8, _keyword: u64, string: *const u16) -> u32 {
    if string.is_null() {
        return etw::ERROR_INVALID_PARAMETER;
    }
    with_table(|t| t.write(reg_handle, 0, false))
}

/// The slot of the thread's activity id (a `LocalAlloc` block of 16 bytes
/// per thread that sets one), allocated at first use.
static ACTIVITY_SLOT: AtomicU32 = AtomicU32::new(u32::MAX);

fn activity_slot() -> Option<u32> {
    let s = ACTIVITY_SLOT.load(Ordering::Acquire);
    if s != u32::MAX {
        return Some(s);
    }
    // SAFETY: no preconditions.
    let new = unsafe { TlsAlloc() };
    if new == u32::MAX {
        return None;
    }
    match ACTIVITY_SLOT.compare_exchange(u32::MAX, new, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(new),
        Err(other) => Some(other),
    }
}

/// `EventActivityIdControl(ControlCode, ActivityId*)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EventActivityIdControl(control: u32, activity_id: *mut u8) -> u32 {
    let Some(slot) = activity_slot() else { return etw::ERROR_NOT_ENOUGH_MEMORY };
    // SAFETY: a value of this process's slot, null or a block of 16 bytes.
    let mut block = unsafe { TlsGetValue(slot) };
    let needs_block = matches!(
        control,
        etw::activity::EVENT_ACTIVITY_CTRL_SET_ID
            | etw::activity::EVENT_ACTIVITY_CTRL_GET_SET_ID
            | etw::activity::EVENT_ACTIVITY_CTRL_CREATE_SET_ID
    );
    if block.is_null() && needs_block {
        // SAFETY: LMEM_ZEROINIT (0x40): 16 zero bytes, the all-zero id.
        block = unsafe { LocalAlloc(0x40, 16) };
        if block.is_null() {
            return etw::ERROR_NOT_ENOUGH_MEMORY;
        }
        // SAFETY: our slot.
        unsafe { TlsSetValue(slot, block) };
    }
    let mut current = [0u8; 16];
    if !block.is_null() {
        // SAFETY: a block of 16 bytes made above.
        unsafe { core::ptr::copy_nonoverlapping(block, current.as_mut_ptr(), 16) };
    }
    let mut random = [0u8; 16];
    if control == etw::activity::EVENT_ACTIVITY_CTRL_CREATE_ID
        || control == etw::activity::EVENT_ACTIVITY_CTRL_CREATE_SET_ID
    {
        // SAFETY: a buffer of 16 bytes.
        unsafe { crate::SystemFunction036(random.as_mut_ptr(), 16) };
    }
    let mut io = [0u8; 16];
    if !activity_id.is_null() {
        // SAFETY: the caller's GUID.
        unsafe { core::ptr::copy_nonoverlapping(activity_id, io.as_mut_ptr(), 16) };
    }
    let r = etw::activity_id_control(
        control,
        if activity_id.is_null() { None } else { Some(&mut io) },
        &mut current,
        random,
    );
    if r == etw::ERROR_SUCCESS {
        if !activity_id.is_null() {
            // SAFETY: the caller's GUID.
            unsafe { core::ptr::copy_nonoverlapping(io.as_ptr(), activity_id, 16) };
        }
        if !block.is_null() {
            // SAFETY: the thread's block of 16 bytes.
            unsafe { core::ptr::copy_nonoverlapping(current.as_ptr(), block, 16) };
        }
    }
    r
}

// ----- the classic API (evntrace.h) ---------------------------------------------------------

/// `RegisterTraceGuidsW(RequestAddress, RequestContext, ControlGuid, GuidCount,
/// TraceGuidReg, MofImagePath, MofResourceName, RegistrationHandle*)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterTraceGuidsW(
    request: usize,
    context: usize,
    control_guid: *const u8,
    _guid_count: u32,
    _trace_guid_reg: *const u8,
    _mof_image_path: *const u16,
    _mof_resource_name: *const u16,
    registration: *mut u64,
) -> u32 {
    if request == 0 || control_guid.is_null() || registration.is_null() {
        return etw::ERROR_INVALID_PARAMETER;
    }
    match with_table(|t| t.register(guid_at(control_guid), etw::Kind::Classic, request, context)) {
        Ok(h) => {
            // SAFETY: checked non-null.
            unsafe { registration.write(h) };
            etw::ERROR_SUCCESS
        }
        Err(e) => e,
    }
}

/// `UnregisterTraceGuids(RegistrationHandle)`.
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterTraceGuids(registration: u64) -> u32 {
    with_table(|t| t.unregister(registration)).err().unwrap_or(etw::ERROR_SUCCESS)
}

/// `TraceEvent(TraceHandle, EVENT_TRACE_HEADER*)`: the header's first `USHORT` is its `Size`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TraceEvent(_trace_handle: u64, header: *const u8) -> u32 {
    let size = if header.is_null() {
        None
    } else {
        // SAFETY: the caller's header, at least its first two bytes.
        Some(unsafe { core::ptr::read_unaligned(header.cast::<u16>()) })
    };
    etw::trace_event(size)
}

/// `GetTraceEnableFlags(TraceHandle)`: no session, no flags.
#[unsafe(no_mangle)]
pub extern "system" fn GetTraceEnableFlags(_trace_handle: u64) -> u32 {
    0
}

/// `GetTraceEnableLevel(TraceHandle)`: no session, level 0.
#[unsafe(no_mangle)]
pub extern "system" fn GetTraceEnableLevel(_trace_handle: u64) -> u8 {
    0
}

/// `GetTraceLoggerHandle(Buffer)`: the `HistoricalContext` of the `WNODE_HEADER` a
/// provider's callback was given, or `INVALID_HANDLE_VALUE`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTraceLoggerHandle(buffer: *const u8) -> u64 {
    let bytes = if buffer.is_null() {
        None
    } else {
        // SAFETY: a WNODE_HEADER (48 bytes) as the callback received it.
        Some(unsafe { core::slice::from_raw_parts(buffer, 16) })
    };
    match etw::trace_logger_handle(bytes) {
        Ok(h) => h,
        Err(e) => {
            // SAFETY: no preconditions.
            unsafe { SetLastError(e) };
            etw::INVALID_TRACEHANDLE
        }
    }
}
