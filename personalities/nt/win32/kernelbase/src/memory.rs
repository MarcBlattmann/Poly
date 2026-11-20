//! Memory (Learn's `memoryapi.h`, `heapapi.h` and the local-memory pages
//! of `winbase.h`): virtual memory over `NtAllocateVirtualMemory` and
//! friends (ADR 0063), heaps over our `ntdll`'s NT heap (ADR 0071).

use crate::base::*;
use crate::nt;
use poly_kernelbase::error::*;
use poly_kernelbase::memory::{self, *};

/// `VirtualAlloc`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualAlloc(address: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8 {
    if let Err(e) = memory::check_alloc_type(kind) {
        return fail(e, core::ptr::null_mut());
    }
    let (mut base, mut sz) = (address, size);
    // SAFETY: valid in/out pointers; the kernel checks the arguments.
    let st = unsafe { nt::NtAllocateVirtualMemory(nt::CURRENT_PROCESS, &mut base, 0, &mut sz, kind, protect) };
    if st.is_success() { base } else { fail_status(st, core::ptr::null_mut()) }
}

/// `VirtualFree`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualFree(address: *mut u8, size: usize, kind: u32) -> i32 {
    if let Err(e) = memory::check_free(size, kind) {
        return fail(e, FALSE);
    }
    let (mut base, mut sz) = (address, size);
    // SAFETY: valid in/out pointers.
    let st = unsafe { nt::NtFreeVirtualMemory(nt::CURRENT_PROCESS, &mut base, &mut sz, kind) };
    if st.is_success() {
        // What was freed is locked no more (`VirtualLock`).
        crate::b0::forget_locked(base as usize, sz);
        TRUE
    } else {
        fail_status(st, FALSE)
    }
}

/// `VirtualProtect`: Learn: `lpflOldProtect` must point at a variable.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualProtect(address: *mut u8, size: usize, protect: u32, old: *mut u32) -> i32 {
    if old.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    let (mut base, mut sz) = (address, size);
    // SAFETY: valid in/out pointers.
    let st = unsafe { nt::NtProtectVirtualMemory(nt::CURRENT_PROCESS, &mut base, &mut sz, protect, old) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `VirtualQuery`: the bytes of `MEMORY_BASIC_INFORMATION` written, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualQuery(address: *const u8, info: *mut u8, length: usize) -> usize {
    // SAFETY: as VirtualQueryEx.
    unsafe { VirtualQueryEx(nt::CURRENT_PROCESS, address, info, length) }
}

/// `VirtualAllocEx`: `VirtualAlloc` in the process `process` names (Learn: the handle needs
/// `PROCESS_VM_OPERATION`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualAllocEx(
    process: nt::Handle,
    address: *mut u8,
    size: usize,
    kind: u32,
    protect: u32,
) -> *mut u8 {
    if let Err(e) = memory::check_alloc_type(kind) {
        return fail(e, core::ptr::null_mut());
    }
    let (mut base, mut sz) = (address, size);
    // SAFETY: valid in/out pointers; the kernel checks the arguments.
    let st = unsafe { nt::NtAllocateVirtualMemory(process, &mut base, 0, &mut sz, kind, protect) };
    if st.is_success() { base } else { fail_status(st, core::ptr::null_mut()) }
}

/// `VirtualFreeEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualFreeEx(process: nt::Handle, address: *mut u8, size: usize, kind: u32) -> i32 {
    if let Err(e) = memory::check_free(size, kind) {
        return fail(e, FALSE);
    }
    let (mut base, mut sz) = (address, size);
    // SAFETY: valid in/out pointers.
    let st = unsafe { nt::NtFreeVirtualMemory(process, &mut base, &mut sz, kind) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `VirtualProtectEx`: Learn: `lpflOldProtect` must point at a variable.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualProtectEx(
    process: nt::Handle,
    address: *mut u8,
    size: usize,
    protect: u32,
    old: *mut u32,
) -> i32 {
    if old.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    // A size of 0 is refused before anything changes (the sheet: FALSE, `ERROR_INVALID_PARAMETER`,
    // the old protection untouched).
    if size == 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let (mut base, mut sz) = (address, size);
    // SAFETY: valid in/out pointers.
    let st = unsafe { nt::NtProtectVirtualMemory(process, &mut base, &mut sz, protect, old) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `FlushInstructionCache(process, address, size)` (Learn: "Flushes the instruction cache for the
/// specified process"): x86-64 keeps the instruction cache coherent with writes, so for a process
/// handle all it does is accept it (Chrome's sandbox calls it after patching a target: the sheet
/// of `VirtualProtectEx`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FlushInstructionCache(process: nt::Handle, _address: *const u8, _size: usize) -> i32 {
    if process != nt::CURRENT_PROCESS {
        // SAFETY: `process` is only looked at.
        if unsafe { crate::spawn::GetProcessId(process as _) } == 0 {
            // Not a process handle: GetProcessId set the error (ERROR_INVALID_HANDLE).
            return FALSE;
        }
    }
    TRUE
}

/// `VirtualQueryEx`: the bytes of `MEMORY_BASIC_INFORMATION` written, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn VirtualQueryEx(
    process: nt::Handle,
    address: *const u8,
    info: *mut u8,
    length: usize,
) -> usize {
    let mut returned = 0usize;
    // SAFETY: the caller's buffer of `length` bytes; MemoryBasicInformation
    // (0) has the same layout as MEMORY_BASIC_INFORMATION.
    let st = unsafe { nt::NtQueryVirtualMemory(process, address, 0, info, length, &mut returned) };
    if st.is_success() { returned } else { fail_status(st, 0) }
}

// ----- heaps ----------------------------------------------------------------------------

/// `GetProcessHeap`.
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessHeap() -> *mut u8 {
    process_heap()
}

/// `HeapCreate`: Learn: a maximum of 0 makes a growable heap; failure is
/// NULL with an error.
#[unsafe(no_mangle)]
pub extern "system" fn HeapCreate(options: u32, initial: usize, maximum: usize) -> *mut u8 {
    let flags = memory::heap_create_flags(options, maximum);
    // SAFETY: no caller memory; ntdll reserves the heap.
    let h = unsafe {
        nt::RtlCreateHeap(flags, core::ptr::null_mut(), maximum, initial, core::ptr::null_mut(), core::ptr::null_mut())
    };
    if h.is_null() { fail(ERROR_NOT_ENOUGH_MEMORY, h) } else { h }
}

/// `HeapDestroy`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HeapDestroy(heap: *mut u8) -> i32 {
    if heap.is_null() || heap == process_heap() {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    // SAFETY: a heap the caller created.
    unsafe { nt::RtlDestroyHeap(heap) };
    TRUE
}

/// `HeapAlloc`: Learn: "If the function fails, it does not call
/// SetLastError".
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8 {
    // SAFETY: the caller's heap.
    unsafe { nt::RtlAllocateHeap(heap, flags, bytes) }
}

/// `HeapFree`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HeapFree(heap: *mut u8, flags: u32, ptr: *mut u8) -> i32 {
    // SAFETY: the caller's heap and block.
    if unsafe { nt::RtlFreeHeap(heap, flags, ptr) } != 0 { TRUE } else { fail(ERROR_INVALID_PARAMETER, FALSE) }
}

/// `HeapReAlloc`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HeapReAlloc(heap: *mut u8, flags: u32, ptr: *mut u8, bytes: usize) -> *mut u8 {
    // SAFETY: the caller's heap and block.
    unsafe { nt::RtlReAllocateHeap(heap, flags, ptr, bytes) }
}

/// `HeapSize`: `(SIZE_T)-1` on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HeapSize(heap: *mut u8, flags: u32, ptr: *const u8) -> usize {
    // SAFETY: the caller's heap and block.
    unsafe { nt::RtlSizeHeap(heap, flags, ptr) }
}

// ----- local memory (the process heap; moveable blocks are fixed) ---------------------

/// `LocalAlloc`.
#[unsafe(no_mangle)]
pub extern "system" fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8 {
    let hf = match memory::local_alloc_flags(flags) {
        Ok(f) => f,
        Err(e) => return fail(e, core::ptr::null_mut()),
    };
    // SAFETY: the process heap.
    let p = unsafe { nt::RtlAllocateHeap(process_heap(), hf, bytes) };
    if p.is_null() { fail(ERROR_NOT_ENOUGH_MEMORY, p) } else { p }
}

/// `LocalFree`: NULL on success; the handle back and
/// `ERROR_INVALID_HANDLE` on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LocalFree(mem: *mut u8) -> *mut u8 {
    if mem.is_null() {
        return mem;
    }
    // SAFETY: a block of the process heap.
    if unsafe { nt::RtlFreeHeap(process_heap(), 0, mem) } != 0 {
        core::ptr::null_mut()
    } else {
        fail(ERROR_INVALID_HANDLE, mem)
    }
}

/// `LocalReAlloc`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LocalReAlloc(mem: *mut u8, bytes: usize, flags: u32) -> *mut u8 {
    if flags & LMEM_MODIFY != 0 {
        return mem;
    }
    // SAFETY: a block of the process heap.
    let p = unsafe { nt::RtlReAllocateHeap(process_heap(), memory::local_realloc_flags(flags), mem, bytes) };
    if p.is_null() { fail(ERROR_NOT_ENOUGH_MEMORY, p) } else { p }
}

/// `LocalLock`: a block's address (it never moves).
#[unsafe(no_mangle)]
pub extern "system" fn LocalLock(mem: *mut u8) -> *mut u8 {
    mem
}

/// `LocalUnlock`: Learn: returns FALSE with `NO_ERROR` once the block is
/// unlocked (always, for a fixed block).
#[unsafe(no_mangle)]
pub extern "system" fn LocalUnlock(_mem: *mut u8) -> i32 {
    fail(ERROR_SUCCESS, FALSE)
}

/// `LocalSize`: 0 with an error for a bad handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LocalSize(mem: *mut u8) -> usize {
    // SAFETY: a block of the process heap.
    let n = unsafe { nt::RtlSizeHeap(process_heap(), 0, mem) };
    if n == usize::MAX { fail(ERROR_INVALID_HANDLE, 0) } else { n }
}

// ----- ReadProcessMemory, WriteProcessMemory ---------------------------------------------

/// Copy between this process's memory and `buf`, region by region: a region that cannot be
/// read (or, for a write, opened for writing) ends the copy with `ERROR_PARTIAL_COPY` and the
/// bytes done so far. Another process's memory goes through `NtReadVirtualMemory` /
/// `NtWriteVirtualMemory` (ADR 0194).
unsafe fn copy_self(process: isize, base: usize, buf: *mut u8, size: usize, done: *mut usize, write: bool) -> i32 {
    if process != nt::CURRENT_PROCESS as isize {
        // `process` is only looked at.
        let id = crate::spawn::GetProcessId(process as _);
        if id == 0 {
            // Not a process handle: GetProcessId set the error (ERROR_INVALID_HANDLE).
            return FALSE;
        }
        if id != crate::process::GetCurrentProcessId() {
            // SAFETY: as copy_other.
            return unsafe { copy_other(process, base, buf, size, done, write) };
        }
    }
    let mut copied = 0usize;
    let mut ok = true;
    while copied < size {
        let at = base.wrapping_add(copied);
        let mut info = crate::base::MemoryBasicInformation::default();
        // SAFETY: a MEMORY_BASIC_INFORMATION buffer of this build's layout (28 bytes, 48 in 64 bits).
        let q = unsafe {
            VirtualQuery(
                at as *const u8,
                (&mut info as *mut crate::base::MemoryBasicInformation).cast(),
                core::mem::size_of::<crate::base::MemoryBasicInformation>(),
            )
        };
        if q == 0 {
            ok = false;
            break;
        }
        let (region_base, region_size) = (info.base_address, info.region_size);
        let (state, protect) = (info.state, info.protect);
        if !region_readable(state, protect) {
            ok = false;
            break;
        }
        let n = (size - copied).min(region_base + region_size - at);
        let mut restore = None;
        if write && !protect_writable(protect) {
            let mut old = 0u32;
            // SAFETY: the pages of this process's own region.
            if unsafe { VirtualProtect(at as *mut u8, n, protect_for_write(protect), &mut old) } == FALSE {
                ok = false;
                break;
            }
            restore = Some(old);
        }
        // SAFETY: `n` bytes inside a committed, now accessible region; the caller's buffer holds `size`.
        unsafe {
            if write {
                core::ptr::copy(buf.add(copied), at as *mut u8, n);
            } else {
                core::ptr::copy(at as *const u8, buf.add(copied), n);
            }
        }
        if let Some(old) = restore {
            let mut ignored = 0u32;
            // SAFETY: as above.
            unsafe { VirtualProtect(at as *mut u8, n, old, &mut ignored) };
        }
        copied += n;
    }
    if !done.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *done = copied };
    }
    if ok { TRUE } else { fail(ERROR_PARTIAL_COPY, FALSE) }
}

/// [`copy_self`] for another process: the kernel copies page by page and
/// stops at the first page it cannot read or write (`STATUS_PARTIAL_COPY`,
/// `ERROR_PARTIAL_COPY`).
///
/// A write that stops at a page that is committed but only executable or execute-read (an image's
/// code, a stub the broker patches) is made to work as `WriteProcessMemory` does (the sheet): the
/// page gets a writable protection for the copy and its old one back after it. `PAGE_READONLY`
/// and `PAGE_NOACCESS` pages are `ERROR_NOACCESS` (nothing written); a copy that has written
/// something and then meets memory it cannot write is `ERROR_PARTIAL_COPY`.
///
/// # Safety
/// `buf` holds `size` bytes; `done` is null or writable.
unsafe fn copy_other(process: isize, base: usize, buf: *mut u8, size: usize, done: *mut usize, write: bool) -> i32 {
    let mut copied = 0usize;
    let mut result = TRUE;
    while copied < size {
        let mut n = 0usize;
        let at = base.wrapping_add(copied);
        // SAFETY: the caller's buffer of `size` bytes and a local count.
        let st = unsafe {
            if write {
                nt::NtWriteVirtualMemory(process as nt::Handle, at as *mut u8, buf.add(copied), size - copied, &mut n)
            } else {
                nt::NtReadVirtualMemory(process as nt::Handle, at as *const u8, buf.add(copied), size - copied, &mut n)
            }
        };
        copied += n;
        if st.is_success() {
            break;
        }
        if st.0 != 0x8000_000D || !write {
            // Not a partial copy (access denied, bad handle), or a read: as it is.
            if !done.is_null() {
                // SAFETY: the caller's out parameter.
                unsafe { *done = copied };
            }
            return if st.0 == 0x8000_000D { fail(ERROR_PARTIAL_COPY, FALSE) } else { fail_status(st, FALSE) };
        }
        // STATUS_PARTIAL_COPY on a write: look at the page that stopped it.
        let at = base.wrapping_add(copied);
        let mut info = crate::base::MemoryBasicInformation::default();
        // SAFETY: a MEMORY_BASIC_INFORMATION buffer of this build's layout.
        let q = unsafe {
            VirtualQueryEx(
                process as nt::Handle,
                at as *const u8,
                (&mut info as *mut crate::base::MemoryBasicInformation).cast(),
                core::mem::size_of::<crate::base::MemoryBasicInformation>(),
            )
        };
        let code_page = q != 0
            && info.state == MEM_COMMIT
            && info.protect & (PAGE_NOACCESS | PAGE_GUARD) == 0
            && !protect_writable(info.protect)
            && info.protect & (PAGE_EXECUTE | 0x20) != 0;
        if !code_page {
            result = fail(if copied == 0 { ERROR_NOACCESS } else { ERROR_PARTIAL_COPY }, FALSE);
            break;
        }
        // The rest of the region that is not writable, one piece.
        let piece = (size - copied).min(info.base_address + info.region_size - at);
        // A page of an image is copy-on-write (it gets a private copy); private memory is made read/write.
        let temp = if info.ty == 0x0100_0000 { PAGE_EXECUTE_WRITECOPY } else { PAGE_EXECUTE_READWRITE };
        let mut old = 0u32;
        // SAFETY: valid pointers; the kernel checks the arguments.
        if unsafe { VirtualProtectEx(process as nt::Handle, at as *mut u8, piece, temp, &mut old) } == FALSE {
            result = FALSE;
            break;
        }
        let mut put = 0usize;
        // SAFETY: the caller's buffer holds `size` bytes.
        let st =
            unsafe { nt::NtWriteVirtualMemory(process as nt::Handle, at as *mut u8, buf.add(copied), piece, &mut put) };
        let mut ignored = 0u32;
        // SAFETY: as above: the protection the page had comes back.
        unsafe { VirtualProtectEx(process as nt::Handle, at as *mut u8, piece, old, &mut ignored) };
        copied += put;
        if !st.is_success() {
            result = if st.0 == 0x8000_000D { fail(ERROR_PARTIAL_COPY, FALSE) } else { fail_status(st, FALSE) };
            break;
        }
    }
    if !done.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *done = copied };
    }
    if result == TRUE && copied < size { fail(ERROR_PARTIAL_COPY, FALSE) } else { result }
}

/// `ReadProcessMemory`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReadProcessMemory(
    process: isize,
    base: *const u8,
    buf: *mut u8,
    size: usize,
    read: *mut usize,
) -> i32 {
    // SAFETY: the caller's pointers; copy_self checks the source.
    unsafe { copy_self(process, base as usize, buf, size, read, false) }
}

/// `WriteProcessMemory`: a read-only or code region is made writable for the copy and put back.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WriteProcessMemory(
    process: isize,
    base: *mut u8,
    buf: *const u8,
    size: usize,
    written: *mut usize,
) -> i32 {
    // SAFETY: the caller's pointers; copy_self checks the target.
    unsafe { copy_self(process, base as usize, buf as *mut u8, size, written, true) }
}
