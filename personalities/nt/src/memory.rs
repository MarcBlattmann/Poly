//! The memory, section and process services: arguments in, the model of
//! [`crate::vm`] and [`crate::section`] asked, results out.
//!
//! Every memory service names a process: `NtCurrentProcess()`, or a handle
//! to a process object with the right access, the caller's own or another
//! one (ADR 0169 for views, ADR 0194 for allocation, protection, queries and
//! `NtReadVirtualMemory` / `NtWriteVirtualMemory`, which `ReadProcessMemory`
//! and `WriteProcessMemory` are). A null handle is `STATUS_INVALID_HANDLE`.

use crate::abi;
use crate::consts::*;
use crate::kernel::NtKernel;
use crate::objects::{Result, return_handle};
use crate::section::{self, CreateSection, MapView};
use crate::system::{NtProcess, NtSystem};
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::status::*;
use poly_ntobj::{Caller, Handle, NtStatus, ObjectType};

/// Check that `h` names the calling process with `access`.
/// Whose address space a process handle names.
#[derive(Clone, Copy)]
enum Target {
    Own,
    Other(poly_ntobj::ProcessId),
}

/// The process `h` names, for a service that works on other processes too
/// (ADR 0169): the caller's own, or another one.
fn target_of(sys: &NtSystem, proc: &NtProcess, caller: &Caller, h: Handle, access: u32) -> Result<Target> {
    if h == Handle::CURRENT_PROCESS {
        return Ok(Target::Own);
    }
    if h == Handle::NULL {
        return Err(STATUS_INVALID_HANDLE);
    }
    let id = sys.objects.object_by_handle(caller, h, access, Some(ObjectType::Process))?;
    if proc.object == Some(id) {
        return Ok(Target::Own);
    }
    Ok(Target::Other(poly_ntobj::ProcessId(sys.objects.cookie(id).ok_or(STATUS_INVALID_HANDLE)? as u32)))
}

/// Run `f` on the state and address space of the process `h` names (with
/// `access`): the caller's own, or another one's. A process that has gone
/// is `STATUS_PROCESS_IS_TERMINATING`.
fn on_process<K: NtKernel, R>(
    k: &mut K,
    h: u64,
    access: u32,
    f: impl FnOnce(&mut NtSystem, &mut NtProcess, &mut dyn crate::backend::Backend) -> Result<R>,
) -> Result<R> {
    let caller = k.caller();
    let target = k.with_state(|sys, proc, _| target_of(sys, proc, &caller, Handle(h), access))?;
    match target {
        Target::Own => k.with_state(f),
        Target::Other(pid) => k.with_process(pid, f).ok_or(STATUS_PROCESS_IS_TERMINATING)?,
    }
}

/// `STATUS_DYNAMIC_CODE_BLOCKED` (`ntstatus.h`): a process with the
/// prohibit-dynamic-code mitigation cannot make memory executable.
const STATUS_DYNAMIC_CODE_BLOCKED: NtStatus = NtStatus(0xC000_0604);

/// Whether a `PAGE_*` protection allows execution.
fn executable(protect: u32) -> bool {
    protect & 0xF0 != 0
}

/// `NtAllocateVirtualMemory`: `*BaseAddress` and `*RegionSize` in and out.
pub fn allocate<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, base_ptr, zero_bits, size_ptr, alloc_type, protect] = abi::args::<K, 6>(k, regs)?;
    let base = abi::read_u64(k, base_ptr)?;
    let size = abi::read_u64(k, size_ptr)?;
    let (base, size) = on_process(k, process, PROCESS_VM_OPERATION, |sys, proc, b| {
        if proc.dynamic_code_blocked && alloc_type as u32 & MEM_COMMIT != 0 && executable(protect as u32) {
            return Err(STATUS_DYNAMIC_CODE_BLOCKED);
        }
        let r = proc.vm.allocate(b, base, size, zero_bits, alloc_type as u32, protect as u32)?;
        if alloc_type as u32 & MEM_COMMIT != 0 {
            section::record_section_commit(sys, proc, r.0, r.1);
        }
        Ok(r)
    })?;
    abi::write_u64(k, base_ptr, base)?;
    abi::write_u64(k, size_ptr, size)?;
    Ok(STATUS_SUCCESS)
}

/// `NtFreeVirtualMemory`.
pub fn free<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, base_ptr, size_ptr, free_type] = abi::args::<K, 4>(k, regs)?;
    let base = abi::read_u64(k, base_ptr)?;
    let size = abi::read_u64(k, size_ptr)?;
    let (base, size) =
        on_process(k, process, PROCESS_VM_OPERATION, |_, proc, b| proc.vm.free(b, base, size, free_type as u32))?;
    abi::write_u64(k, base_ptr, base)?;
    abi::write_u64(k, size_ptr, size)?;
    Ok(STATUS_SUCCESS)
}

/// `NtProtectVirtualMemory`: the old protection goes to `*OldProtect`,
/// which is required (`STATUS_ACCESS_VIOLATION` for a null pointer, as for
/// `VirtualProtect`'s `lpflOldProtect`: "If this parameter is NULL ... the
/// function fails"; the status is to be confirmed by the spec agent).
pub fn protect<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, base_ptr, size_ptr, new, old_ptr] = abi::args::<K, 5>(k, regs)?;
    let base = abi::read_u64(k, base_ptr)?;
    let size = abi::read_u64(k, size_ptr)?;
    if old_ptr == 0 {
        return Err(STATUS_ACCESS_VIOLATION);
    }
    let (base, size, old) = on_process(k, process, PROCESS_VM_OPERATION, |_, proc, b| {
        if proc.dynamic_code_blocked && executable(new as u32) {
            return Err(STATUS_DYNAMIC_CODE_BLOCKED);
        }
        proc.vm.protect(b, base, size, new as u32)
    })?;
    abi::write_u64(k, base_ptr, base)?;
    abi::write_u64(k, size_ptr, size)?;
    abi::write_u32(k, old_ptr, old)?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryVirtualMemory(MemoryBasicInformation)`; other classes are
/// `STATUS_INVALID_INFO_CLASS`, a buffer smaller than
/// `MEMORY_BASIC_INFORMATION` `STATUS_INFO_LENGTH_MISMATCH` (to be
/// confirmed by the spec agent).
pub fn query<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, addr, class, buf, length, ret] = abi::args::<K, 6>(k, regs)?;
    if class as u32 == MEMORY_MAPPED_FILENAME_INFORMATION {
        return mapped_filename(k, process, addr, buf, length, ret);
    }
    if class as u32 != MEMORY_BASIC_INFORMATION_CLASS {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    if (length as usize) < MEMORY_BASIC_INFORMATION_SIZE {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let info = on_process(k, process, PROCESS_QUERY_INFORMATION, |_, proc, _| proc.vm.query(addr))?;
    abi::write(k, buf, &info.to_bytes())?;
    if ret != 0 {
        abi::write_u64(k, ret, MEMORY_BASIC_INFORMATION_SIZE as u64)?;
    }
    Ok(STATUS_SUCCESS)
}

/// `STATUS_PARTIAL_COPY` (`ntstatus.h`): `ReadProcessMemory` / `WriteProcessMemory`'s
/// `ERROR_PARTIAL_COPY`.
pub const STATUS_PARTIAL_COPY: NtStatus = NtStatus(0x8000_000D);
/// How much one step of a cross-process copy holds in the kernel.
const COPY_CHUNK: usize = 0x1_0000;

/// Copy page by page with `step` (a read from or a write to the target process) until a page
/// fails: the bytes done.
fn page_steps(at: u64, n: usize, mut step: impl FnMut(u64, core::ops::Range<usize>) -> bool) -> usize {
    let mut done = 0usize;
    while done < n {
        let here = at.wrapping_add(done as u64);
        let page_end = (here | (crate::vm::PAGE_SIZE - 1)).wrapping_add(1);
        let m = (page_end.wrapping_sub(here) as usize).min(n - done);
        if !step(here, done..done + m) {
            break;
        }
        done += m;
    }
    done
}

/// `NtReadVirtualMemory(ProcessHandle, BaseAddress, Buffer, NumberOfBytesToRead,
/// NumberOfBytesRead*)`, `ReadProcessMemory` (Learn: the handle needs `PROCESS_VM_READ`; "the
/// entire area to be read must be accessible": a part that is not ends the copy, which is
/// `STATUS_PARTIAL_COPY` with the bytes done so far).
pub fn read_virtual<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, base, buf, len, done_ptr] = abi::args::<K, 5>(k, regs)?;
    let mut done = 0usize;
    let mut status = STATUS_SUCCESS;
    let mut chunk = Vec::new();
    while done < len as usize {
        let n = (len as usize - done).min(COPY_CHUNK);
        chunk.resize(n, 0);
        let at = base.wrapping_add(done as u64);
        let got = on_process(k, process, PROCESS_VM_READ, |_, _, b| {
            Ok(page_steps(at, n, |addr, r| b.read(addr, &mut chunk[r]).is_ok()))
        })?;
        if got > 0 {
            abi::write(k, buf + done as u64, &chunk[..got])?;
        }
        done += got;
        if got < n {
            status = STATUS_PARTIAL_COPY;
            break;
        }
    }
    if done_ptr != 0 {
        abi::write_u64(k, done_ptr, done as u64)?;
    }
    Ok(status)
}

/// `NtWriteVirtualMemory(ProcessHandle, BaseAddress, Buffer, NumberOfBytesToWrite,
/// NumberOfBytesWritten*)`, `WriteProcessMemory` (Learn: the handle needs `PROCESS_VM_WRITE` and
/// `PROCESS_VM_OPERATION`; a part that cannot be written ends the copy, `STATUS_PARTIAL_COPY`).
pub fn write_virtual<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, base, buf, len, done_ptr] = abi::args::<K, 5>(k, regs)?;
    let mut done = 0usize;
    let mut status = STATUS_SUCCESS;
    let mut chunk = Vec::new();
    while done < len as usize {
        let n = (len as usize - done).min(COPY_CHUNK);
        chunk.resize(n, 0);
        abi::read(k, buf + done as u64, &mut chunk)?;
        let at = base.wrapping_add(done as u64);
        let put = on_process(k, process, PROCESS_VM_WRITE | PROCESS_VM_OPERATION, |_, _, b| {
            Ok(page_steps(at, n, |addr, r| b.write(addr, &chunk[r]).is_ok()))
        })?;
        done += put;
        if put < n {
            status = STATUS_PARTIAL_COPY;
            break;
        }
    }
    if done_ptr != 0 {
        abi::write_u64(k, done_ptr, done as u64)?;
    }
    Ok(status)
}

/// `MemoryMappedFilenameInformation` (`MEMORY_INFORMATION_CLASS`, `ntddk.h`): the name of the file
/// a view maps.
const MEMORY_MAPPED_FILENAME_INFORMATION: u32 = 2;
/// `STATUS_INVALID_ADDRESS`, `STATUS_FILE_INVALID` (`ntstatus.h`).
const STATUS_INVALID_ADDRESS: NtStatus = NtStatus(0xC000_0141);
const STATUS_FILE_INVALID: NtStatus = NtStatus(0xC000_0098);

/// `NtQueryVirtualMemory(MemoryMappedFilenameInformation)`: the object name of the file (device
/// name and path, as `NtQueryObject(ObjectNameInformation)` spells it) behind the view or image
/// at `addr`, as a `MEMORY_SECTION_NAME` (a `UNICODE_STRING` and its characters). An address in no
/// allocation or in private memory is `STATUS_INVALID_ADDRESS`; a view of a section with no file
/// `STATUS_FILE_INVALID` (`specs/windows/kernel32.dll/K32GetMappedFileNameW.yaml`). The process
/// handle needs `PROCESS_QUERY_LIMITED_INFORMATION`.
fn mapped_filename<K: NtKernel>(
    k: &mut K,
    process: u64,
    addr: u64,
    buf: u64,
    length: u64,
    ret: u64,
) -> Result<NtStatus> {
    let caller = k.caller();
    // PROCESS_QUERY_INFORMATION covers what PROCESS_QUERY_LIMITED_INFORMATION does
    // (`K32GetMappedFileNameW.yaml` `process_handle`).
    let target = k.with_state(|sys, proc, _| {
        match target_of(sys, proc, &caller, Handle(process), PROCESS_QUERY_LIMITED_INFORMATION) {
            Err(STATUS_ACCESS_DENIED) => target_of(sys, proc, &caller, Handle(process), PROCESS_QUERY_INFORMATION),
            other => other,
        }
    })?;
    let (device, path) = match target {
        Target::Own => k.with_state(|sys, proc, _| mapped_file_of(sys, proc, addr))?,
        // Another process's views are as readable as the caller's own
        // (`K32GetMappedFileNameW.yaml` `process_handle`).
        Target::Other(pid) => k
            .with_process(pid, |sys, proc, _| mapped_file_of(sys, proc, addr))
            .ok_or(STATUS_PROCESS_IS_TERMINATING)??,
    };
    let a = crate::objects::file_name_answer(&device, path.as_deref(), length as usize, buf);
    abi::write_opt_u32(k, ret, a.return_length)?;
    abi::write(k, buf, &a.bytes)?;
    Ok(a.status)
}

/// The device name and the path of the file the view or image at `addr` of `proc` maps.
fn mapped_file_of(sys: &NtSystem, proc: &NtProcess, addr: u64) -> Result<(Vec<u16>, Option<Vec<u16>>)> {
    let view = proc.vm.allocation(addr).and_then(|a| a.view()).ok_or(STATUS_INVALID_ADDRESS)?;
    let file = match &sys.section(view.section).ok_or(STATUS_INVALID_ADDRESS)?.kind {
        crate::system::SectionKind::File { file } => Some(*file),
        crate::system::SectionKind::Image { file: None, image } => {
            // An image the kernel mapped from bytes: the name it was started from.
            return image.name.clone().map(|(device, path)| (device, Some(path))).ok_or(STATUS_FILE_INVALID);
        }
        crate::system::SectionKind::Image { file, .. } => *file,
        crate::system::SectionKind::Pagefile { .. } => None,
    };
    let cookie = sys.objects.cookie(file.ok_or(STATUS_FILE_INVALID)?).ok_or(STATUS_FILE_INVALID)?;
    sys.io.object_name_parts(cookie).ok_or(STATUS_FILE_INVALID)
}

/// `NtCreateSection`.
pub fn create_section<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, attrs, size_ptr, protect, attributes, file] = abi::args::<K, 7>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let maximum_size = if size_ptr == 0 { None } else { Some(abi::read_u64(k, size_ptr)?) };
    let caller = k.caller();
    let created = k.with_state(|sys, _, b| {
        let oa = attrs.as_object_attributes();
        let req = CreateSection {
            attrs: &oa,
            desired: access as u32,
            maximum_size,
            page_protection: protect as u32,
            allocation_attributes: attributes as u32,
            file: (file != 0).then_some(Handle(file)),
        };
        section::create_section(sys, b, &caller, &req)
    })?;
    return_handle(k, out, created.handle)?;
    Ok(created.status)
}

/// `NtOpenSection`.
pub fn open_section<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [out, access, attrs] = abi::args::<K, 3>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    let caller = k.caller();
    let h =
        k.with_state(|sys, _, _| section::open_section(sys, &caller, &attrs.as_object_attributes(), access as u32))?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtMapViewOfSection`: `*BaseAddress`, `*SectionOffset` (optional) and
/// `*ViewSize` in and out.
pub fn map_view<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [sec, process, base_ptr, zero_bits, commit_size, offset_ptr, size_ptr, inherit, alloc_type, protect] =
        abi::args::<K, 10>(k, regs)?;
    let base = abi::read_u64(k, base_ptr)?;
    let offset = if offset_ptr == 0 { None } else { Some(abi::read_u64(k, offset_ptr)?) };
    let view_size = abi::read_u64(k, size_ptr)?;
    let req = MapView {
        section: Handle(sec),
        base,
        zero_bits,
        commit_size,
        offset,
        view_size,
        inherit: inherit as u32,
        allocation_type: alloc_type as u32,
        protect: protect as u32,
    };
    let caller = k.caller();
    let target = k.with_state(|sys, proc, _| target_of(sys, proc, &caller, Handle(process), PROCESS_VM_OPERATION))?;
    let mapped = match target {
        Target::Own => k.with_state(|sys, proc, b| section::map_view(sys, proc, b, &caller, &req, false))?,
        Target::Other(pid) => k
            .with_process(pid, |sys, proc, b| section::map_view(sys, proc, b, &caller, &req, false))
            .ok_or(STATUS_PROCESS_IS_TERMINATING)??,
    };
    let written = abi::write_u64(k, base_ptr, mapped.base)
        .and_then(|()| abi::write_u64(k, size_ptr, mapped.size))
        .and_then(|()| if offset_ptr != 0 { abi::write_u64(k, offset_ptr, mapped.offset) } else { Ok(()) });
    if let Err(e) = written {
        match target {
            Target::Own => {
                let _ = k.with_state(|sys, proc, b| section::unmap_view(sys, proc, b, mapped.base));
            }
            Target::Other(pid) => {
                let _ = k.with_process(pid, |sys, proc, b| section::unmap_view(sys, proc, b, mapped.base));
            }
        }
        return Err(e);
    }
    Ok(mapped.status)
}

/// `NtUnmapViewOfSectionEx(ProcessHandle, BaseAddress, Flags)`: `Flags` may hold
/// `MEM_UNMAP_WITH_TRANSIENT_BOOST` (a scheduling hint, ignored) and `MEM_PRESERVE_PLACEHOLDER`
/// (Learn, "UnmapViewOfFile2": the view, which replaced a placeholder, becomes that placeholder
/// again; see [`section::unmap_view_ex`]). Other flags:
/// `STATUS_INVALID_PARAMETER_3`. Statuses to be confirmed by the spec agent.
pub fn unmap_view_ex<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, addr, flags] = abi::args::<K, 3>(k, regs)?;
    let flags = flags as u32;
    if flags & !(MEM_UNMAP_WITH_TRANSIENT_BOOST | MEM_PRESERVE_PLACEHOLDER) != 0 {
        return Err(STATUS_INVALID_PARAMETER_3);
    }
    let preserve = flags & MEM_PRESERVE_PLACEHOLDER != 0;
    let caller = k.caller();
    let target = k.with_state(|sys, proc, _| target_of(sys, proc, &caller, Handle(process), PROCESS_VM_OPERATION))?;
    match target {
        Target::Own => k.with_state(|sys, proc, b| section::unmap_view_ex(sys, proc, b, addr, preserve))?,
        Target::Other(pid) => k
            .with_process(pid, |sys, proc, b| section::unmap_view_ex(sys, proc, b, addr, preserve))
            .ok_or(STATUS_PROCESS_IS_TERMINATING)??,
    };
    Ok(STATUS_SUCCESS)
}

/// `MEM_UNMAP_WITH_TRANSIENT_BOOST` (`winnt.h`).
const MEM_UNMAP_WITH_TRANSIENT_BOOST: u32 = 1;

/// `NtUnmapViewOfSection`.
pub fn unmap_view<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, addr] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let target = k.with_state(|sys, proc, _| target_of(sys, proc, &caller, Handle(process), PROCESS_VM_OPERATION))?;
    match target {
        Target::Own => k.with_state(|sys, proc, b| section::unmap_view(sys, proc, b, addr))?,
        Target::Other(pid) => k
            .with_process(pid, |sys, proc, b| section::unmap_view(sys, proc, b, addr))
            .ok_or(STATUS_PROCESS_IS_TERMINATING)??,
    };
    Ok(STATUS_SUCCESS)
}

/// `NtQuerySection(SectionHandle, SectionInformationClass,
/// SectionInformation, SectionInformationLength, ReturnLength*)`: class 0,
/// `SECTION_BASIC_INFORMATION` (`wdm.h`): the base address (0), the
/// allocation attributes and the maximum size, 24 bytes (ADR 0169).
pub fn query_section<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, len, ret] = abi::args::<K, 5>(k, regs)?;
    let caller = k.caller();
    let info = k.with_state(|sys, _, _| -> Result<[u8; 24]> {
        let access = poly_ntobj::access::SECTION_QUERY;
        let obj = sys.objects.object_by_handle(&caller, Handle(h), access, Some(ObjectType::Section))?;
        let key = sys.objects.cookie(obj).ok_or(STATUS_INVALID_HANDLE)?;
        let s = sys.section(key).ok_or(STATUS_INVALID_HANDLE)?;
        let class = class as u32;
        if class > 4 {
            return Err(STATUS_INVALID_INFO_CLASS);
        }
        if class != 0 {
            // `SectionImageInformation` (64 bytes), `SectionRelocationInformation`
            // and `SectionOriginalBaseInformation` (8): data sections are no images.
            let need = if class == 1 { 64 } else { 8 };
            if (len as u32 as usize) < need {
                return Err(STATUS_INFO_LENGTH_MISMATCH);
            }
            return Err(STATUS_SECTION_NOT_IMAGE);
        }
        // SEC_FILE for a section of a file, SEC_IMAGE for an image, else
        // what the section was made with (`SEC_COMMIT`, `SEC_RESERVE`).
        let attributes = match &s.kind {
            crate::system::SectionKind::File { .. } => 0x0080_0000u32,
            crate::system::SectionKind::Image { .. } => 0x0100_0000u32,
            crate::system::SectionKind::Pagefile { .. } => s.attributes & 0x0C00_0000,
        };
        let mut b = [0u8; 24];
        b[8..12].copy_from_slice(&attributes.to_le_bytes());
        b[16..24].copy_from_slice(&s.size.to_le_bytes());
        Ok(b)
    })?;
    if (len as u32) < 24 {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    abi::write(k, buf, &info)?;
    if ret != 0 {
        abi::write_u32(k, ret, 24)?;
    }
    Ok(STATUS_SUCCESS)
}

/// `NtFlushVirtualMemory(ProcessHandle, BaseAddress*, RegionSize*,
/// IoStatusBlock*)` (not in the WDK; Poly's reading of `FlushViewOfFile`
/// with the address and size by reference, rounded to pages and written
/// back, as in `NtProtectVirtualMemory`, ADR 0079). The range must be in
/// one view (`STATUS_NOT_MAPPED_VIEW`); a size of 0 runs to the end of the
/// view. A shared view of a file maps the file's cached pages, so what was
/// written is the file's contents already; the page cache's writeback puts
/// it on disk (Poly does not force it out here, to be confirmed by the
/// spec agent what Windows waits for).
pub fn flush<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, base_ptr, size_ptr, iosb] = abi::args::<K, 4>(k, regs)?;
    let addr = abi::read_u64(k, base_ptr)?;
    let size = abi::read_u64(k, size_ptr)?;
    let (base, len) = on_process(k, process, PROCESS_VM_OPERATION, |_, proc, _| {
        let a = proc.vm.allocation(addr).filter(|a| a.view().is_some()).ok_or(STATUS_NOT_MAPPED_VIEW)?;
        let start = addr & !(crate::vm::PAGE_SIZE - 1);
        let end = if size == 0 {
            a.base + a.size
        } else {
            crate::vm::page_up(addr.checked_add(size).ok_or(STATUS_INVALID_PARAMETER)?)
                .ok_or(STATUS_INVALID_PARAMETER)?
        };
        if end > a.base + a.size {
            return Err(STATUS_NOT_MAPPED_VIEW);
        }
        Ok((start, end - start))
    })?;
    abi::write_u64(k, base_ptr, base)?;
    abi::write_u64(k, size_ptr, len)?;
    if iosb != 0 {
        let mut b = [0u8; 16];
        b[8..16].copy_from_slice(&len.to_le_bytes());
        abi::write(k, iosb, &b)?;
    }
    Ok(STATUS_SUCCESS)
}

/// `NtTerminateProcess`. `NtCurrentProcess()` (or a handle to the caller's
/// own process with `PROCESS_TERMINATE`) ends the calling process with
/// `ExitStatus` once the call returns. A null handle ends every other
/// thread of the caller's process (none can exist yet) and returns.
/// Other processes: `STATUS_NOT_IMPLEMENTED` until the process milestone.
pub fn terminate<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [process, status] = abi::args::<K, 2>(k, regs)?;
    if process == Handle::NULL.0 {
        return Ok(STATUS_SUCCESS);
    }
    let caller = k.caller();
    let status = NtStatus(status as u32);
    // Another process (ADR 0169): its threads are made to end.
    let other = k.with_state(|sys, proc, _| -> Result<Option<poly_ntobj::ProcessId>> {
        if Handle(process) == Handle::CURRENT_PROCESS {
            proc.exit_status.get_or_insert(status);
            return Ok(None);
        }
        let id =
            sys.objects.object_by_handle(&caller, Handle(process), PROCESS_TERMINATE, Some(ObjectType::Process))?;
        if proc.object == Some(id) {
            // The first termination's status is the process's (ADR 0079).
            proc.exit_status.get_or_insert(status);
            return Ok(None);
        }
        let pid = poly_ntobj::ProcessId(sys.objects.cookie(id).expect("live process object") as u32);
        if sys.process_exits.contains_key(&pid) {
            return Err(STATUS_PROCESS_IS_TERMINATING);
        }
        Ok(Some(pid))
    })?;
    match other {
        Some(pid) => k.terminate_other_process(pid, status)?,
        None => k.terminate_process(status),
    }
    Ok(STATUS_SUCCESS)
}
