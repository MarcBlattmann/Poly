//! Our `iphlpapi.dll` (ADR 0155): the interface and address queries
//! Chrome's network service makes (`GetAdaptersAddresses`, `GetIfTable2`,
//! `NotifyAddrChange`). Poly's interfaces come from `\Device\Afd`
//! (`IF_LIST`, via `poly-win32-netrt`); the memory layouts are
//! `poly-iphlp`'s (host-tested). Address changes never happen (Poly's
//! addresses are the boot setting), so a notification stays pending until
//! it is cancelled.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_iphlp::{self as ip, oldapi};
use poly_win32_netrt as rt;
use rt::Handle;

const TRUE: i32 = 1;
const FALSE: i32 = 0;
/// `ERROR_NOT_FOUND`, `ERROR_NETWORK_UNREACHABLE` (`winerror.h`).
const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_NETWORK_UNREACHABLE: u32 = 1231;
/// `STATUS_CANCELLED` (`ntstatus.h`).
const STATUS_CANCELLED: usize = 0xC000_0120;
/// `OVERLAPPED` (`minwinbase.h`): `hEvent` is at 24.
const OVERLAPPED_INTERNAL: usize = 0;
const OVERLAPPED_EVENT: usize = 24;

// ----- GetAdaptersAddresses and the older calls -----------------------------------------

/// Copy `bytes` to the caller's buffer.
unsafe fn copy_out(bytes: &[u8], to: *mut u8) {
    // SAFETY: the caller's buffer, checked to hold at least `bytes.len()`.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), to, bytes.len()) };
}

/// `GetAdaptersAddresses(Family, Flags, Reserved, AdapterAddresses, SizePointer)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAdaptersAddresses(
    family: u32,
    flags: u32,
    reserved: *mut u8,
    addresses: *mut u8,
    size: *mut u32,
) -> u32 {
    if size.is_null() || !reserved.is_null() {
        return ip::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: checked non-null.
    let capacity = if addresses.is_null() { 0 } else { unsafe { size.read() } };
    match ip::adapters::call(&rt::interfaces(), family, flags, addresses as u64, capacity) {
        Ok(bytes) => {
            // SAFETY: `capacity` covered `bytes`.
            unsafe { copy_out(&bytes, addresses) };
            ip::NO_ERROR
        }
        Err((e, need)) => {
            if e == ip::ERROR_BUFFER_OVERFLOW {
                // SAFETY: checked non-null.
                unsafe { size.write(need) };
            }
            e
        }
    }
}

/// `GetAdaptersInfo(AdapterInfo, SizePointer)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAdaptersInfo(info: *mut u8, size: *mut u32) -> u32 {
    if size.is_null() {
        return ip::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: checked non-null.
    let capacity = if info.is_null() { 0 } else { unsafe { size.read() } };
    match oldapi::adapter_info_call(&rt::interfaces(), info as u64, capacity) {
        Ok(bytes) => {
            // SAFETY: `capacity` covered `bytes`.
            unsafe { copy_out(&bytes, info) };
            ip::NO_ERROR
        }
        Err((e, need)) => {
            if e == ip::ERROR_BUFFER_OVERFLOW {
                // SAFETY: checked non-null.
                unsafe { size.write(need) };
            }
            e
        }
    }
}

/// `GetInterfaceInfo(IfTable, SizePointer)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetInterfaceInfo(info: *mut u8, size: *mut u32) -> u32 {
    if size.is_null() {
        return ip::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: checked non-null.
    let capacity = if info.is_null() { 0 } else { unsafe { size.read() } };
    match oldapi::interface_info_call(&rt::interfaces(), capacity) {
        Ok(bytes) => {
            // SAFETY: `capacity` covered `bytes`.
            unsafe { copy_out(&bytes, info) };
            ip::NO_ERROR
        }
        Err((e, need)) => {
            // SAFETY: checked non-null.
            unsafe { size.write(need) };
            e
        }
    }
}

/// `GetNumberOfInterfaces(NumInterfaces*)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNumberOfInterfaces(n: *mut u32) -> u32 {
    if n.is_null() {
        return ip::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: checked non-null.
    unsafe { n.write(rt::interfaces().len() as u32) };
    ip::NO_ERROR
}

/// `IpReleaseAddress` / `IpRenewAddress`: Poly's addresses are static.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IpReleaseAddress(_map: *const u8) -> u32 {
    ip::ERROR_NOT_SUPPORTED
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn IpRenewAddress(_map: *const u8) -> u32 {
    ip::ERROR_NOT_SUPPORTED
}

// ----- GetIfTable2 / FreeMibTable ---------------------------------------------------------

/// `GetIfTable2(Table**)`: the table is allocated here and freed by `FreeMibTable`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetIfTable2(table: *mut *mut u8) -> u32 {
    if table.is_null() {
        return ip::ERROR_INVALID_PARAMETER;
    }
    let bytes = ip::ifrow::table(&rt::interfaces());
    let p = rt::heap_copy(&bytes);
    if p.is_null() {
        return ip::ERROR_NOT_ENOUGH_MEMORY;
    }
    // SAFETY: checked non-null.
    unsafe { table.write(p) };
    ip::NO_ERROR
}

/// `FreeMibTable(Memory)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeMibTable(memory: *mut u8) {
    // SAFETY: a table from `GetIfTable2`, or null.
    unsafe { rt::heap_free(memory) };
}

// ----- index and name ----------------------------------------------------------------------

/// `if_indextoname(InterfaceIndex, InterfaceName[IF_NAMESIZE = 256])`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn if_indextoname(index: u32, name: *mut u8) -> *mut u8 {
    if name.is_null() {
        return core::ptr::null_mut();
    }
    match oldapi::index_to_name(&rt::interfaces(), index) {
        Some(n) => {
            // SAFETY: the caller's buffer of IF_NAMESIZE bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(n.as_ptr(), name, n.len());
                name.add(n.len()).write(0);
            }
            name
        }
        None => core::ptr::null_mut(),
    }
}

/// `if_nametoindex(InterfaceName)`: 0 for an unknown name.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn if_nametoindex(name: *const u8) -> u32 {
    if name.is_null() {
        return 0;
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string.
    let s = unsafe {
        while *name.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(name, n)
    };
    core::str::from_utf8(s).map_or(0, |s| oldapi::name_to_index(&rt::interfaces(), s))
}

// ----- the best interface -------------------------------------------------------------------

/// `GetBestInterface(dwDestAddr, pdwBestIfIndex)`: `dwDestAddr` is the address as it lies in memory
/// (network order bytes in a little-endian `DWORD`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetBestInterface(dest: u32, index: *mut u32) -> u32 {
    if index.is_null() {
        return ip::ERROR_INVALID_PARAMETER;
    }
    match oldapi::best_interface_v4(&rt::interfaces(), dest.to_le_bytes()) {
        Some(i) => {
            // SAFETY: checked non-null.
            unsafe { index.write(i) };
            ip::NO_ERROR
        }
        None => ERROR_NETWORK_UNREACHABLE,
    }
}

/// `GetBestInterfaceEx(pDestAddr: SOCKADDR*, pdwBestIfIndex)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetBestInterfaceEx(dest: *const u8, index: *mut u32) -> u32 {
    if dest.is_null() || index.is_null() {
        return ip::ERROR_INVALID_PARAMETER;
    }
    // SAFETY: a SOCKADDR_IN or SOCKADDR_IN6 (the family says which).
    let best = unsafe {
        match core::ptr::read_unaligned(dest.cast::<u16>()) {
            2 => {
                let mut a = [0u8; 4];
                core::ptr::copy_nonoverlapping(dest.add(4), a.as_mut_ptr(), 4);
                oldapi::best_interface_v4(&rt::interfaces(), a)
            }
            23 => {
                let mut a = [0u8; 16];
                core::ptr::copy_nonoverlapping(dest.add(8), a.as_mut_ptr(), 16);
                oldapi::best_interface_v6(&rt::interfaces(), a)
            }
            _ => return ip::ERROR_INVALID_PARAMETER,
        }
    };
    match best {
        Some(i) => {
            // SAFETY: checked non-null.
            unsafe { index.write(i) };
            ip::NO_ERROR
        }
        None => ERROR_NETWORK_UNREACHABLE,
    }
}

// ----- address-change notification --------------------------------------------------------

/// The `OVERLAPPED`s waiting for an address change (as addresses), behind a spin lock.
struct Pending {
    locked: AtomicBool,
    list: core::cell::UnsafeCell<Vec<usize>>,
}

// SAFETY: access to `list` is serialised by `locked`.
unsafe impl Sync for Pending {}

static PENDING: Pending = Pending { locked: AtomicBool::new(false), list: core::cell::UnsafeCell::new(Vec::new()) };

fn with_pending<R>(f: impl FnOnce(&mut Vec<usize>) -> R) -> R {
    while PENDING.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let r = f(unsafe { &mut *PENDING.list.get() });
    PENDING.locked.store(false, Ordering::Release);
    r
}

/// `NotifyAddrChange(Handle*, OVERLAPPED*)`: with both given, the request is asynchronous
/// (`ERROR_IO_PENDING`; the overlapped's event is signalled at a change, or when the request is
/// cancelled). Without them the call waits for a change, which never comes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn NotifyAddrChange(handle: *mut Handle, overlapped: *mut u8) -> u32 {
    if overlapped.is_null() {
        // Synchronous: wait for ever on an event nobody signals.
        // SAFETY: plain kernel32 calls.
        unsafe {
            let ev = rt::CreateEventW(core::ptr::null(), 1, 0, core::ptr::null());
            if ev != 0 {
                rt::WaitForSingleObject(ev, u32::MAX);
                rt::CloseHandle(ev);
            }
        }
        return ip::NO_ERROR;
    }
    // SAFETY: the caller's OVERLAPPED.
    let event = unsafe { core::ptr::read_unaligned(overlapped.add(OVERLAPPED_EVENT).cast::<Handle>()) };
    if !handle.is_null() {
        // SAFETY: the caller's variable. Windows hands back a handle for the request; the
        // overlapped's event (or 1) identifies it here.
        unsafe { handle.write(if event != 0 { event } else { 1 }) };
    }
    with_pending(|l| l.push(overlapped as usize));
    ip::ERROR_IO_PENDING
}

/// `CancelIPChangeNotify(OVERLAPPED*)`: the request is completed as cancelled and its event set.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CancelIPChangeNotify(overlapped: *mut u8) -> i32 {
    if overlapped.is_null() {
        rt::set_error(ip::ERROR_INVALID_PARAMETER);
        return FALSE;
    }
    let found = with_pending(|l| match l.iter().position(|&p| p == overlapped as usize) {
        Some(i) => {
            l.remove(i);
            true
        }
        None => false,
    });
    if !found {
        rt::set_error(ERROR_NOT_FOUND);
        return FALSE;
    }
    // SAFETY: the caller's OVERLAPPED, still valid while the request was pending (Learn).
    unsafe {
        core::ptr::write_unaligned(overlapped.add(OVERLAPPED_INTERNAL).cast::<usize>(), STATUS_CANCELLED);
        let event = core::ptr::read_unaligned(overlapped.add(OVERLAPPED_EVENT).cast::<Handle>());
        if event != 0 {
            rt::SetEvent(event);
        }
    }
    TRUE
}

/// `NotifyRouteChange(Handle*, OVERLAPPED*)`: the same, for routes (which never change either).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn NotifyRouteChange(handle: *mut Handle, overlapped: *mut u8) -> u32 {
    // SAFETY: the caller's arguments.
    unsafe { NotifyAddrChange(handle, overlapped) }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn iphlpapi_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    TRUE
}
