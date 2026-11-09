//! System information, locale, time zone, loader and memory functions of
//! Chrome's batch 2 (ADR 0155): Learn's pages named in
//! `poly_kernelbase::sysex` and `::tz`. The rules and structures are
//! those modules'; this is the glue.

use crate::base::*;
use crate::nt::{self, Handle};
use alloc::vec::Vec;
use poly_kernelbase::error::*;
use poly_kernelbase::sysex::{self, AllocRequest};
use poly_kernelbase::time;
use poly_kernelbase::tz;
use poly_ntobj::NtStatus;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn LdrAddRefDll(flags: u32, handle: *mut u8) -> NtStatus;
    fn RtlPcToFileHeader(pc: *mut u8, base: *mut *mut u8) -> *mut u8;
}

/// `ERROR_BUFFER_OVERFLOW` (`winerror.h`).
const ERROR_BUFFER_OVERFLOW: u32 = 111;

// ----- GetModuleHandleExA / W -------------------------------------------------------------

/// `GetModuleHandleExW`: Learn: the handle is stored in `*phModule` (NULL on
/// failure); unless `GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT` the
/// module's reference count goes up (`..._PIN` keeps it loaded for good);
/// `..._FROM_ADDRESS` takes an address inside the module instead of a name.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetModuleHandleExW(flags: u32, name: *const u16, out: *mut *mut u8) -> i32 {
    if out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's out-pointer.
    unsafe { out.write(core::ptr::null_mut()) };
    let f = match sysex::module_ex_flags(flags) {
        Ok(f) => f,
        Err(e) => return fail(e, FALSE),
    };
    let module = if f.from_address {
        let mut base = core::ptr::null_mut();
        // SAFETY: an address is only looked up.
        let m = unsafe { RtlPcToFileHeader(name as *mut u8, &mut base) };
        if m.is_null() {
            return fail(ERROR_MOD_NOT_FOUND, FALSE);
        }
        m
    } else {
        // SAFETY: the caller's string (NULL: the executable).
        let m = unsafe { crate::process::GetModuleHandleW(name) };
        if m.is_null() {
            return FALSE; // GetModuleHandleW set the error (ERROR_MOD_NOT_FOUND).
        }
        m
    };
    if !f.unchanged {
        // SAFETY: a module handle the loader returned.
        unsafe { LdrAddRefDll(u32::from(f.pin), module) };
    }
    // SAFETY: the caller's out-pointer.
    unsafe { out.write(module) };
    TRUE
}

/// `GetModuleHandleExA`: the name in the ANSI code page.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetModuleHandleExA(flags: u32, name: *const u8, out: *mut *mut u8) -> i32 {
    if flags & sysex::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS != 0 {
        // SAFETY: an address, not a string.
        return unsafe { GetModuleHandleExW(flags, name.cast(), out) };
    }
    // SAFETY: the caller's string.
    let wide = unsafe { ansi_arg(name) }.map(|w| with_nul(&w));
    // SAFETY: NUL-terminated, or null.
    unsafe { GetModuleHandleExW(flags, wide.as_ref().map_or(core::ptr::null(), |w| w.as_ptr()), out) }
}

/// `WerRegisterRuntimeExceptionModule`: Learn: registers a DLL to be called
/// when the process crashes. Poly has no error reporting; the module is
/// validated (a path is required) and forgotten. An `HRESULT`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WerRegisterRuntimeExceptionModule(path: *const u16, _context: *mut u8) -> i32 {
    // SAFETY: the caller's string.
    match unsafe { wide_arg(path) } {
        Some(p) if !p.is_empty() => 0,
        _ => 0x8007_0057u32 as i32, // E_INVALIDARG = HRESULT_FROM_WIN32(ERROR_INVALID_PARAMETER)
    }
}

/// `WerUnregisterRuntimeExceptionModule`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WerUnregisterRuntimeExceptionModule(path: *const u16, context: *mut u8) -> i32 {
    // SAFETY: as above.
    unsafe { WerRegisterRuntimeExceptionModule(path, context) }
}

// ----- memory ---------------------------------------------------------------------------------

/// `PAGE_NOACCESS` (`winnt.h`).
const PAGE_NOACCESS: u32 = 1;
/// `MEM_RESERVE`, `MEM_RELEASE`, `MEM_RESET`.
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const MEM_RESET: u32 = 0x8_0000;
/// `MEM_REPLACE_PLACEHOLDER`, `MEM_RESERVE_PLACEHOLDER` (`winnt.h`).
const MEM_REPLACE_PLACEHOLDER: u32 = 0x4000;
const MEM_RESERVE_PLACEHOLDER: u32 = 0x4_0000;
/// `MEM_UNMAP_WITH_TRANSIENT_BOOST`, `MEM_PRESERVE_PLACEHOLDER`.
const MEM_UNMAP_WITH_TRANSIENT_BOOST: u32 = 1;
const MEM_PRESERVE_PLACEHOLDER: u32 = 2;
/// `ViewShare` (`wdm.h`'s `SECTION_INHERIT`).
const VIEW_SHARE: u32 = 1;

fn process_of(h: Handle) -> Handle {
    if h == 0 { nt::CURRENT_PROCESS } else { h }
}

/// `DiscardVirtualMemory`: Learn: discards the contents of committed pages
/// (their next read is zeros); the result is `ERROR_SUCCESS` or an error
/// code (not through `GetLastError`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DiscardVirtualMemory(address: *mut u8, size: usize) -> u32 {
    let (mut base, mut sz) = (address, size);
    // SAFETY: valid in/out pointers; `MEM_RESET` keeps the pages committed.
    let st =
        unsafe { nt::NtAllocateVirtualMemory(nt::CURRENT_PROCESS, &mut base, 0, &mut sz, MEM_RESET, PAGE_NOACCESS) };
    if st.is_success() { ERROR_SUCCESS } else { from_status(st) }
}

/// `PrefetchVirtualMemory`: a hint; Poly maps memory on demand and ignores
/// it after checking the arguments (no flags are defined: they must be 0).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PrefetchVirtualMemory(
    _process: Handle,
    count: usize,
    entries: *const u8,
    flags: u32,
) -> i32 {
    if flags != 0 || count == 0 || entries.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    TRUE
}

/// `HeapSetInformation`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HeapSetInformation(heap: *mut u8, class: u32, info: *mut u8, length: usize) -> i32 {
    if heap.is_null() && class != sysex::HEAP_ENABLE_TERMINATION_ON_CORRUPTION {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let bytes = if info.is_null() {
        if length != 0 {
            return fail(ERROR_INVALID_PARAMETER, FALSE);
        }
        None
    } else {
        // SAFETY: the caller's buffer of `length` bytes.
        Some(unsafe { core::slice::from_raw_parts(info, length) })
    };
    match sysex::heap_set_information(class, bytes) {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

/// The address requirements behind a `MEM_EXTENDED_PARAMETER` (read from
/// the caller's memory).
fn read_requirements(ptr: u64) -> Option<[u8; sysex::MEM_ADDRESS_REQUIREMENTS_SIZE]> {
    if ptr == 0 {
        return None;
    }
    // SAFETY: the caller promised a MEM_ADDRESS_REQUIREMENTS here.
    Some(unsafe { (ptr as *const [u8; sysex::MEM_ADDRESS_REQUIREMENTS_SIZE]).read_unaligned() })
}

/// The request of the `count` extended parameters at `params`.
///
/// # Safety
/// `params` is null or `count` `MEM_EXTENDED_PARAMETER`s.
unsafe fn request_of(params: *const u8, count: u32) -> Result<AllocRequest, u32> {
    if count != 0 && params.is_null() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let raw: &[u8] = if count == 0 {
        &[]
    } else {
        // SAFETY: the caller's array.
        unsafe { core::slice::from_raw_parts(params, count as usize * sysex::MEM_EXTENDED_PARAMETER_SIZE) }
    };
    sysex::alloc_request(raw, count as usize, &read_requirements)
}

/// Find where `size` bytes obeying `req` fit and `take` them: `take(at)`
/// tries to place the allocation or view at `at` and says whether it
/// worked. With only an alignment, a trial reservation finds a free range
/// to align inside; with bounds, aligned candidates are probed.
fn place(
    process: Handle,
    req: &AllocRequest,
    size: usize,
    take: &mut dyn FnMut(usize) -> Result<*mut u8, NtStatus>,
) -> Result<*mut u8, u32> {
    let align = sysex::effective_alignment(req);
    let mut last = NtStatus(0xC000_0017); // STATUS_NO_MEMORY
    if req.lowest == 0 && req.highest == 0 {
        for _ in 0..16 {
            let (mut base, mut trial) = (core::ptr::null_mut::<u8>(), size + align);
            // SAFETY: valid in/out pointers.
            let st =
                unsafe { nt::NtAllocateVirtualMemory(process, &mut base, 0, &mut trial, MEM_RESERVE, PAGE_NOACCESS) };
            if !st.is_success() {
                return Err(from_status(st));
            }
            let at = sysex::align_up(base as usize, align);
            let mut zero = 0usize;
            // SAFETY: the trial reservation made above.
            unsafe { nt::NtFreeVirtualMemory(process, &mut base, &mut zero, MEM_RELEASE) };
            match take(at) {
                Ok(p) => return Ok(p),
                Err(st) => last = st,
            }
        }
        return Err(from_status(last));
    }
    let step = sysex::align_up(size, align).max(align);
    let mut at = sysex::align_up(req.lowest.max(0x1_0000), align);
    for _ in 0..4096 {
        if !sysex::range_ok(req, at, size) {
            break;
        }
        match take(at) {
            Ok(p) => return Ok(p),
            Err(st) => last = st,
        }
        at += step;
    }
    let _ = last;
    Err(ERROR_NOT_ENOUGH_MEMORY)
}

/// `VirtualAlloc2`: `VirtualAlloc` in another process, with extended
/// parameters (address requirements are honoured; NUMA nodes and attribute
/// flags are ignored; partitions and user-physical pages are
/// `ERROR_NOT_SUPPORTED`). A placeholder (`MEM_RESERVE_PLACEHOLDER`) is an
/// ordinary `PAGE_NOACCESS` reservation; `MEM_REPLACE_PLACEHOLDER` commits
/// (or keeps) the reservation at the address.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualAlloc2(
    process: Handle,
    address: *mut u8,
    size: usize,
    kind: u32,
    protect: u32,
    params: *mut u8,
    count: u32,
) -> *mut u8 {
    let process = process_of(process);
    // SAFETY: the caller's array.
    let req = match unsafe { request_of(params, count) } {
        Ok(r) => r,
        Err(e) => return fail(e, core::ptr::null_mut()),
    };
    let mut kind = kind;
    if kind & MEM_RESERVE_PLACEHOLDER != 0 {
        if protect != PAGE_NOACCESS || kind & MEM_RESERVE == 0 {
            return fail(ERROR_INVALID_PARAMETER, core::ptr::null_mut());
        }
        kind &= !MEM_RESERVE_PLACEHOLDER;
    }
    if kind & MEM_REPLACE_PLACEHOLDER != 0 {
        // The placeholder is the reservation already there: commit in it.
        if address.is_null() || size == 0 {
            return fail(ERROR_INVALID_PARAMETER, core::ptr::null_mut());
        }
        kind &= !(MEM_REPLACE_PLACEHOLDER | MEM_RESERVE);
        if kind == 0 {
            return address;
        }
    }
    if address.is_null() && (req.alignment != 0 || req.lowest != 0 || req.highest != 0) {
        let mut take = |at: usize| {
            let (mut base, mut sz) = (at as *mut u8, size);
            // SAFETY: valid in/out pointers.
            let st = unsafe { nt::NtAllocateVirtualMemory(process, &mut base, 0, &mut sz, kind, protect) };
            if st.is_success() { Ok(base) } else { Err(st) }
        };
        return place(process, &req, size, &mut take).unwrap_or_else(|e| fail(e, core::ptr::null_mut()));
    }
    if !address.is_null() && !sysex::range_ok(&req, address as usize, size) && count != 0 {
        return fail(ERROR_INVALID_PARAMETER, core::ptr::null_mut());
    }
    let (mut base, mut sz) = (address, size);
    // SAFETY: valid in/out pointers; the kernel checks the arguments.
    let st = unsafe { nt::NtAllocateVirtualMemory(process, &mut base, 0, &mut sz, kind, protect) };
    if st.is_success() { base } else { fail_status(st, core::ptr::null_mut()) }
}

/// `MapViewOfFile3`: `MapViewOfFile` in another process, with extended
/// parameters. `MEM_REPLACE_PLACEHOLDER` maps over a reservation at
/// `address` (the reservation is released and the view mapped there).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MapViewOfFile3(
    mapping: Handle,
    process: Handle,
    address: *mut u8,
    offset: u64,
    size: usize,
    kind: u32,
    protect: u32,
    params: *mut u8,
    count: u32,
) -> *mut u8 {
    let process = process_of(process);
    // SAFETY: the caller's array.
    let req = match unsafe { request_of(params, count) } {
        Ok(r) => r,
        Err(e) => return fail(e, core::ptr::null_mut()),
    };
    let replace = kind & MEM_REPLACE_PLACEHOLDER != 0;
    if replace && (address.is_null() || size == 0) {
        return fail(ERROR_INVALID_PARAMETER, core::ptr::null_mut());
    }
    let alloc = kind & !MEM_REPLACE_PLACEHOLDER & !MEM_RESERVE_PLACEHOLDER;
    let map_at = |at: usize| -> Result<*mut u8, NtStatus> {
        let (mut base, mut sz, mut off) = (at as *mut u8, size, offset as i64);
        // SAFETY: valid in/out pointers.
        let st = unsafe {
            nt::NtMapViewOfSection(mapping, process, &mut base, 0, 0, &mut off, &mut sz, VIEW_SHARE, alloc, protect)
        };
        if st.is_success() { Ok(base) } else { Err(st) }
    };
    if replace {
        let (mut base, mut sz) = (address, size);
        // SAFETY: the reservation the caller holds at `address`.
        let st = unsafe { nt::NtFreeVirtualMemory(process, &mut base, &mut sz, MEM_RELEASE) };
        if !st.is_success() {
            return fail_status(st, core::ptr::null_mut());
        }
        return match map_at(address as usize) {
            Ok(p) => p,
            Err(st) => {
                // Put the reservation back.
                let (mut b, mut s) = (address, size);
                // SAFETY: valid in/out pointers.
                unsafe { nt::NtAllocateVirtualMemory(process, &mut b, 0, &mut s, MEM_RESERVE, PAGE_NOACCESS) };
                fail_status(st, core::ptr::null_mut())
            }
        };
    }
    if address.is_null() && (req.alignment != 0 || req.lowest != 0 || req.highest != 0) {
        let mut take = |at: usize| map_at(at);
        return place(process, &req, size.max(sysex::PAGE_SIZE), &mut take)
            .unwrap_or_else(|e| fail(e, core::ptr::null_mut()));
    }
    match map_at(address as usize) {
        Ok(p) => p,
        Err(st) => fail_status(st, core::ptr::null_mut()),
    }
}

/// `UnmapViewOfFile2`: `MEM_PRESERVE_PLACEHOLDER` leaves the address range
/// reserved (a placeholder).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnmapViewOfFile2(process: Handle, base: *const u8, flags: u32) -> i32 {
    if flags & !(MEM_UNMAP_WITH_TRANSIENT_BOOST | MEM_PRESERVE_PLACEHOLDER) != 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let process = process_of(process);
    // The view's size, for re-reserving it.
    let mut info = [0u8; 48];
    let mut region = 0usize;
    if flags & MEM_PRESERVE_PLACEHOLDER != 0 {
        // SAFETY: valid pointers; MEMORY_BASIC_INFORMATION is 48 bytes.
        let st = unsafe { nt::NtQueryVirtualMemory(process, base, 0, info.as_mut_ptr(), 48, core::ptr::null_mut()) };
        if st.is_success() {
            region = u64::from_le_bytes(info[24..32].try_into().unwrap()) as usize;
        }
    }
    // SAFETY: an address; the kernel checks it.
    let st = unsafe { nt::NtUnmapViewOfSection(process, base) };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    if region != 0 {
        let (mut b, mut s) = (base as *mut u8, region);
        // SAFETY: valid in/out pointers.
        unsafe { nt::NtAllocateVirtualMemory(process, &mut b, 0, &mut s, MEM_RESERVE, PAGE_NOACCESS) };
    }
    TRUE
}

// ----- locale, geography ---------------------------------------------------------------------------

/// Copy `text` with a NUL into `buf` of `cch` units the way the locale
/// functions answer (see `locale.rs`).
unsafe fn answer(text: &str, buf: *mut u16, cch: i32) -> i32 {
    let units: Vec<u16> = text.encode_utf16().collect();
    let need = units.len() as i32 + 1;
    if cch == 0 {
        return need;
    }
    if cch < need || buf.is_null() {
        return fail(ERROR_INSUFFICIENT_BUFFER, 0);
    }
    // SAFETY: the caller's buffer of `cch >= need` units.
    unsafe { put_wide(buf, &units) };
    need
}

/// `GetLocaleInfoEx`: `GetLocaleInfoW` for a locale name Poly has.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLocaleInfoEx(name: *const u16, lctype: u32, buf: *mut u16, cch: i32) -> i32 {
    // SAFETY: the caller's string.
    if !sysex::locale_name_ok(unsafe { wide_arg(name) }) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // SAFETY: the caller's buffer.
    unsafe { crate::locale::GetLocaleInfoW(poly_kernelbase::locale::LCID_EN_US, lctype, buf, cch) }
}

/// `IsValidLocaleName`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsValidLocaleName(name: *const u16) -> i32 {
    // SAFETY: the caller's string. Learn: NULL is not a valid name.
    match unsafe { wide_arg(name) } {
        Some(n) => i32::from(sysex::locale_name_ok(Some(n))),
        None => fail(ERROR_INVALID_PARAMETER, FALSE),
    }
}

/// `GetUserGeoID`: the `GEOID`, or `GEOID_NOT_AVAILABLE` (-1).
#[unsafe(no_mangle)]
pub extern "system" fn GetUserGeoID(class: u32) -> i32 {
    match sysex::user_geo_id(class) {
        Some(id) => id,
        None => fail(ERROR_INVALID_PARAMETER, sysex::GEOID_NOT_AVAILABLE),
    }
}

/// `GetGeoInfoW`: the length with the NUL, the size needed (`cchData` 0), or
/// 0 with `ERROR_INVALID_FLAGS` for a location or type Poly does not answer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetGeoInfoW(location: i32, geo_type: u32, data: *mut u16, cch: i32, _lang: u16) -> i32 {
    match sysex::geo_info(location, geo_type) {
        // SAFETY: the caller's buffer.
        Some(t) => unsafe { answer(t, data, cch) },
        None => fail(ERROR_INVALID_FLAGS, 0),
    }
}

// ----- time zones ----------------------------------------------------------------------------------------

fn utc_now() -> u64 {
    let mut t = 0i64;
    // SAFETY: a valid pointer.
    unsafe { nt::NtQuerySystemTime(&mut t) };
    t as u64
}

/// `GetTimeZoneInformation`: Poly's zone is UTC (`poly_kernelbase::tz`);
/// the result is `TIME_ZONE_ID_UNKNOWN` (the zone has no daylight saving
/// time).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTimeZoneInformation(out: *mut u8) -> u32 {
    if out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, tz::TIME_ZONE_ID_INVALID);
    }
    let z = tz::utc();
    let b = z.to_bytes();
    // SAFETY: the caller's TIME_ZONE_INFORMATION (172 bytes).
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
    tz::zone_id(&z, utc_now())
}

/// `GetDynamicTimeZoneInformation`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDynamicTimeZoneInformation(out: *mut u8) -> u32 {
    if out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, tz::TIME_ZONE_ID_INVALID);
    }
    let z = tz::utc();
    let b = tz::dynamic_bytes(&z, tz::UTC_KEY_NAME);
    // SAFETY: the caller's DYNAMIC_TIME_ZONE_INFORMATION (432 bytes).
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
    tz::zone_id(&z, utc_now())
}

/// The zone of a `TIME_ZONE_INFORMATION*` argument (NULL: the active one).
///
/// # Safety
/// `p` is null or a `TIME_ZONE_INFORMATION`.
unsafe fn zone_arg(p: *const u8) -> tz::Tzi {
    if p.is_null() {
        return tz::utc();
    }
    // SAFETY: the caller's structure.
    let b = unsafe { core::slice::from_raw_parts(p, tz::TZI_SIZE) };
    tz::Tzi::from_bytes(b).unwrap_or_else(tz::utc)
}

/// Convert a `SYSTEMTIME` between UTC and the zone's local time.
unsafe fn convert_time(zone: &tz::Tzi, input: *const u8, out: *mut u8, to_local: bool) -> i32 {
    if input.is_null() || out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's SYSTEMTIMEs (16 bytes).
    let st = time::SystemTime::from_bytes(&unsafe { (input as *const [u8; 16]).read_unaligned() });
    match tz::convert(zone, &st, to_local) {
        Some(r) => {
            // SAFETY: as above.
            unsafe { (out as *mut [u8; 16]).write_unaligned(r.to_bytes()) };
            TRUE
        }
        None => fail(ERROR_INVALID_PARAMETER, FALSE),
    }
}

/// `SystemTimeToTzSpecificLocalTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemTimeToTzSpecificLocalTime(zone: *const u8, utc: *const u8, local: *mut u8) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { convert_time(&zone_arg(zone), utc, local, true) }
}

/// `TzSpecificLocalTimeToSystemTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TzSpecificLocalTimeToSystemTime(zone: *const u8, local: *const u8, utc: *mut u8) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { convert_time(&zone_arg(zone), local, utc, false) }
}

/// `SystemTimeToTzSpecificLocalTimeEx` (`DYNAMIC_TIME_ZONE_INFORMATION*`;
/// its first 172 bytes are a `TIME_ZONE_INFORMATION`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemTimeToTzSpecificLocalTimeEx(
    zone: *const u8,
    utc: *const u8,
    local: *mut u8,
) -> i32 {
    // SAFETY: as above.
    unsafe { SystemTimeToTzSpecificLocalTime(zone, utc, local) }
}

/// `TzSpecificLocalTimeToSystemTimeEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TzSpecificLocalTimeToSystemTimeEx(
    zone: *const u8,
    local: *const u8,
    utc: *mut u8,
) -> i32 {
    // SAFETY: as above.
    unsafe { TzSpecificLocalTimeToSystemTime(zone, local, utc) }
}

// ----- processors and the computer ------------------------------------------------------------------------------

fn topology() -> sysex::Topology {
    sysex::Topology { cpus: crate::misc::processors() }
}

/// Copy `data` to `buf` if `*length` allows, else `ERROR_INSUFFICIENT_BUFFER`
/// with the size needed; either way `*length` is the size of the data.
unsafe fn fill(buf: *mut u8, length: *mut u32, data: &[u8]) -> i32 {
    if length.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's in/out length.
    let have = unsafe { length.read_unaligned() } as usize;
    // SAFETY: as above.
    unsafe { length.write_unaligned(data.len() as u32) };
    if let Err(e) = sysex::fits(if buf.is_null() { 0 } else { have }, data.len()) {
        return fail(e, FALSE);
    }
    // SAFETY: the caller's buffer holds `have >= data.len()` bytes.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len()) };
    TRUE
}

/// `GetLogicalProcessorInformation`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLogicalProcessorInformation(buf: *mut u8, length: *mut u32) -> i32 {
    // SAFETY: the caller's buffer and length.
    unsafe { fill(buf, length, &topology().legacy()) }
}

/// `GetLogicalProcessorInformationEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetLogicalProcessorInformationEx(
    relationship: u32,
    buf: *mut u8,
    length: *mut u32,
) -> i32 {
    match topology().ex(relationship) {
        // SAFETY: the caller's buffer and length.
        Ok(data) => unsafe { fill(buf, length, &data) },
        Err(e) => fail(e, FALSE),
    }
}

/// The computer's configured name: `COMPUTERNAME`, else `POLY`.
fn host_name() -> alloc::string::String {
    let w = poly_kernelbase::wide;
    crate::process::env_var(&w("COMPUTERNAME"))
        .filter(|n| !n.is_empty())
        .map(|n| char::decode_utf16(n).map(|c| c.unwrap_or('\u{FFFD}')).collect())
        .unwrap_or_else(|| "POLY".into())
}

/// Write the name `kind` for `GetComputerName[Ex]W`, with `overflow` as the
/// error of a buffer that is too small.
unsafe fn computer_name(kind: u32, buf: *mut u16, size: *mut u32, overflow: u32) -> i32 {
    if size.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let name = match sysex::computer_name(kind, &host_name()) {
        Ok(n) => n,
        Err(e) => return fail(e, FALSE),
    };
    let units: Vec<u16> = name.encode_utf16().collect();
    // SAFETY: the caller's in/out size.
    let capacity = unsafe { size.read_unaligned() };
    match sysex::name_fits(units.len(), if buf.is_null() { 0 } else { capacity }) {
        Ok(n) => {
            // SAFETY: the buffer holds more than `units.len()` units.
            unsafe {
                put_wide(buf, &units);
                size.write_unaligned(n);
            }
            TRUE
        }
        Err(needed) => {
            // SAFETY: the caller's in/out size.
            unsafe { size.write_unaligned(needed) };
            fail(overflow, FALSE)
        }
    }
}

/// `GetComputerNameExW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetComputerNameExW(kind: u32, buf: *mut u16, size: *mut u32) -> i32 {
    // SAFETY: the caller's buffer and size.
    unsafe { computer_name(kind, buf, size, ERROR_MORE_DATA) }
}

/// `GetComputerNameW`: the NetBIOS name; a buffer that is too small is
/// `ERROR_BUFFER_OVERFLOW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetComputerNameW(buf: *mut u16, size: *mut u32) -> i32 {
    // SAFETY: as above.
    unsafe { computer_name(sysex::COMPUTER_NAME_NETBIOS, buf, size, ERROR_BUFFER_OVERFLOW) }
}

/// `IMAGE_FILE_MACHINE_UNKNOWN`, `IMAGE_FILE_MACHINE_AMD64` (`winnt.h`).
const IMAGE_FILE_MACHINE_UNKNOWN: u16 = 0;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

/// `IsWow64Process2`: Poly runs 64-bit x64 programs only, so the process
/// is not emulated (`IMAGE_FILE_MACHINE_UNKNOWN`) and the machine is x64.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsWow64Process2(
    _process: Handle,
    process_machine: *mut u16,
    native_machine: *mut u16,
) -> i32 {
    if process_machine.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's out-pointers.
    unsafe {
        process_machine.write_unaligned(IMAGE_FILE_MACHINE_UNKNOWN);
        if !native_machine.is_null() {
            native_machine.write_unaligned(IMAGE_FILE_MACHINE_AMD64);
        }
    }
    TRUE
}

/// `GetSystemPowerStatus`: AC power, no battery.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSystemPowerStatus(out: *mut u8) -> i32 {
    if out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let s = sysex::power_status();
    // SAFETY: the caller's SYSTEM_POWER_STATUS (12 bytes).
    unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), out, s.len()) };
    TRUE
}

/// 100 ns units since the system started, not counting time asleep (Poly
/// never sleeps).
fn interrupt_time() -> u64 {
    let (mut c, mut f) = (0i64, 0i64);
    // SAFETY: valid pointers.
    unsafe { nt::NtQueryPerformanceCounter(&mut c, &mut f) };
    (u128::from(c as u64) * 10_000_000 / u128::from((f as u64).max(1))) as u64
}

/// `QueryUnbiasedInterruptTimePrecise` (`realtimeapiset.h`): no result.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryUnbiasedInterruptTimePrecise(out: *mut u64) {
    if !out.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { out.write_unaligned(interrupt_time()) };
    }
}

/// `QueryUnbiasedInterruptTime`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryUnbiasedInterruptTime(out: *mut u64) -> i32 {
    if out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's out-pointer.
    unsafe { out.write_unaligned(interrupt_time()) };
    TRUE
}
