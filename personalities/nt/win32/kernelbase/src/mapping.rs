//! File mappings (Learn's `memoryapi.h` pages: `CreateFileMappingW`,
//! `OpenFileMappingW`, `MapViewOfFile`, `MapViewOfFileEx`,
//! `UnmapViewOfFile`, `FlushViewOfFile`), ADR 0079's second slice, over
//! the section services (`NtCreateSection`, `NtMapViewOfSection`, ...).
//! The argument rules are `poly_kernelbase::mapping`'s.

use crate::base::*;
use crate::nt::{self, Handle, ObjectAttributes, UnicodeString};
use poly_kernelbase::error::*;
use poly_kernelbase::mapping;
use poly_kernelbase::sync::object_name;

/// `ViewShare` (`wdm.h`'s `SECTION_INHERIT`).
const VIEW_SHARE: u32 = 1;

/// `CreateFileMappingW`: a section of `file` (or of the paging file for
/// `INVALID_HANDLE_VALUE`). A name that exists opens that mapping and
/// sets `ERROR_ALREADY_EXISTS` (Learn). `lpFileMappingAttributes`: Learn:
/// it "determines whether a returned handle can be inherited by child
/// processes", and its `lpSecurityDescriptor` "specifies a security
/// descriptor for a new file mapping object" (ADR 0155, step 9: Chrome
/// gives its shared memory a descriptor so that a read-only handle to it
/// cannot be duplicated into a writable one).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateFileMappingW(
    file: Handle,
    security: *const u8,
    protect: u32,
    size_high: u32,
    size_low: u32,
    name: *const u16,
) -> Handle {
    let size = (u64::from(size_high) << 32) | u64::from(size_low);
    let paging = file == INVALID_HANDLE_VALUE;
    let args = match mapping::create_args(protect, paging, size) {
        Ok(a) => a,
        Err(e) => return fail(e, 0),
    };
    // SAFETY: the caller's string.
    let nt_name = unsafe { wide_arg(name) }.and_then(object_name);
    let us = nt_name.as_deref().map(UnicodeString::new);
    let mut attrs = if us.is_some() { nt::OBJ_OPENIF } else { 0 };
    // SECURITY_ATTRIBUTES (minwinbase.h): lpSecurityDescriptor at 8, bInheritHandle at 16.
    // SAFETY: the caller's structure when non-null.
    let (descriptor, inherit) = if security.is_null() {
        (core::ptr::null(), false)
    } else {
        unsafe { ((security.add(8) as *const *const u8).read(), (security.add(16) as *const i32).read() != 0) }
    };
    if inherit {
        attrs |= nt::OBJ_INHERIT;
    }
    let mut oa = ObjectAttributes::new(us.as_ref(), attrs);
    oa.security_descriptor = descriptor;
    let max = size as i64;
    let mut h: Handle = 0;
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtCreateSection(
            &mut h,
            args.access,
            &oa,
            if size == 0 { core::ptr::null() } else { &max },
            args.protect,
            args.attributes,
            if paging { 0 } else { file },
        )
    };
    if !st.is_success() {
        return fail_status(st, 0);
    }
    set_last_error(if st == nt::STATUS_OBJECT_NAME_EXISTS { ERROR_ALREADY_EXISTS } else { ERROR_SUCCESS });
    h
}

/// `CreateFileMappingA`: the name in the ANSI code page, then as
/// `CreateFileMappingW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateFileMappingA(
    file: Handle,
    security: *const u8,
    protect: u32,
    size_high: u32,
    size_low: u32,
    name: *const u8,
) -> Handle {
    // SAFETY: the caller's string.
    let wide = unsafe { ansi_arg(name) }.map(|w| with_nul(&w));
    let p = wide.as_ref().map_or(core::ptr::null(), |w| w.as_ptr());
    // SAFETY: a NUL-terminated name or null; the rest as given.
    unsafe { CreateFileMappingW(file, security, protect, size_high, size_low, p) }
}

/// `OpenFileMappingW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle {
    // SAFETY: the caller's string.
    let Some(nt_name) = (unsafe { wide_arg(name) }).and_then(object_name) else {
        return fail(ERROR_INVALID_PARAMETER, 0);
    };
    let us = UnicodeString::new(&nt_name);
    let oa = ObjectAttributes::new(Some(&us), if inherit != 0 { nt::OBJ_INHERIT } else { 0 });
    let mut h: Handle = 0;
    // FILE_MAP_EXECUTE is SECTION_MAP_EXECUTE_EXPLICIT; the section needs
    // SECTION_MAP_EXECUTE for it.
    let mut want = access & !mapping::FILE_MAP_EXECUTE;
    if access & mapping::FILE_MAP_EXECUTE != 0 {
        want |= mapping::SECTION_MAP_EXECUTE;
    }
    // SAFETY: valid pointers.
    let st = unsafe { nt::NtOpenSection(&mut h, want, &oa) };
    if st.is_success() { h } else { fail_status(st, 0) }
}

/// `MapViewOfFileEx`: `base` NULL lets the system choose.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MapViewOfFileEx(
    mapping_handle: Handle,
    access: u32,
    offset_high: u32,
    offset_low: u32,
    bytes: usize,
    base: *mut u8,
) -> *mut u8 {
    let (protect, offset) = match (mapping::view_protect(access), mapping::view_offset(offset_high, offset_low)) {
        (Ok(p), Ok(o)) => (p, o),
        (Err(e), _) | (_, Err(e)) => return fail(e, core::ptr::null_mut()),
    };
    let (mut at, mut size, mut off) = (base, bytes, offset as i64);
    // SAFETY: valid in/out pointers.
    let st = unsafe {
        nt::NtMapViewOfSection(
            mapping_handle,
            nt::CURRENT_PROCESS,
            &mut at,
            0,
            0,
            &mut off,
            &mut size,
            VIEW_SHARE,
            0,
            protect,
        )
    };
    if st.is_success() { at } else { fail_status(st, core::ptr::null_mut()) }
}

/// `MapViewOfFile`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MapViewOfFile(
    mapping_handle: Handle,
    access: u32,
    offset_high: u32,
    offset_low: u32,
    bytes: usize,
) -> *mut u8 {
    // SAFETY: as `MapViewOfFileEx`.
    unsafe { MapViewOfFileEx(mapping_handle, access, offset_high, offset_low, bytes, core::ptr::null_mut()) }
}

/// `UnmapViewOfFile`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnmapViewOfFile(base: *const u8) -> i32 {
    // SAFETY: an address; the kernel checks it.
    let st = unsafe { nt::NtUnmapViewOfSection(nt::CURRENT_PROCESS, base) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `FlushViewOfFile`: `bytes` 0 flushes to the end of the view.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FlushViewOfFile(base: *const u8, bytes: usize) -> i32 {
    let (mut at, mut size) = (base as *mut u8, bytes);
    let mut iosb = nt::IoStatusBlock::default();
    // SAFETY: valid in/out pointers.
    let st = unsafe { nt::NtFlushVirtualMemory(nt::CURRENT_PROCESS, &mut at, &mut size, &mut iosb) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}
