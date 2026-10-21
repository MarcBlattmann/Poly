//! Starting an NT process from a PE executable, ending it, and what a CPU
//! exception in it means.
//!
//! Until our `ntdll` exists the kernel starts the executable's entry point
//! itself, so it does here what `ntdll`'s loader will later share with it:
//!
//! 1. The process gets its handle table and a process object; its first
//!    thread a thread object (`NtCurrentProcess()`, `NtCurrentThread()`).
//! 2. The executable becomes an image section, mapped at its preferred base,
//!    or, if that is taken and it has relocations, anywhere and relocated
//!    (`poly_pe::relocate`). Imports are not bound: an executable that
//!    imports anything is refused until `ntdll` does that.
//! 3. The PEB (one page) and the first TEB (two pages) are private memory,
//!    allocated top down. The stack is a reservation of the image's
//!    `SizeOfStackReserve` (64 KiB at least) with `SizeOfStackCommit`
//!    committed at its top and a guard page below that, so it grows on
//!    demand ([`crate::vm`]).
//! 4. The thread starts at the entry point with RSP 8 bytes below a 16-byte
//!    boundary, as after a `call` (Learn: "x64 calling convention",
//!    "Stack allocation"), with 32 bytes of shadow space above the
//!    (zero) return address, and GS based at its TEB.
//!
//! # The TEB and PEB this milestone fills in
//!
//! Only fields whose offsets public headers give:
//!
//! | Field | Offset | Header | Value |
//! |---|---|---|---|
//! | `NT_TIB.StackBase` | TEB + 0x08 | `winnt.h` (`NT_TIB`) | top of the stack |
//! | `NT_TIB.StackLimit` | TEB + 0x10 | `winnt.h` | lowest committed stack address above the guard page (moves as the stack grows) |
//! | `NT_TIB.Self` | TEB + 0x30 | `winnt.h` (`NtCurrentTeb` reads `gs:[0x30]`) | the TEB |
//! | `TEB.ProcessEnvironmentBlock` | TEB + 0x60 | `winternl.h` (after `Reserved1[12]`) | the PEB |
//! | `PEB.BeingDebugged` | PEB + 0x02 | `winternl.h` | 0 |
//!
//! Everything else is zero; the full TEB and PEB are the next milestone's.

use crate::backend::{Backend, SectionSource};
use crate::consts::*;
use crate::image::ImageSection;
use crate::protect::Protection;
use crate::section::{self, MapView};
use crate::system::{NtProcess, NtSystem, NtThread, Section, SectionKind};
use crate::vm::{ALLOCATION_GRANULARITY, AccessFault, PAGE_SIZE, page_up};
use core::result::Result;
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NewObject, NtStatus, ObjectId, ObjectType, ThreadRef};

/// Bytes of a TEB (the `winternl.h` layout is 0x1838 bytes on x64).
pub const TEB_SIZE: u64 = 2 * PAGE_SIZE;
pub const TEB_STACK_BASE: u64 = 0x08;
pub const TEB_STACK_LIMIT: u64 = 0x10;
pub const TEB_SELF: u64 = 0x30;
pub const TEB_PEB: u64 = 0x60;
pub const PEB_BEING_DEBUGGED: u64 = 0x02;

/// Where a new process's first thread starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessStart {
    pub entry: u64,
    pub rsp: u64,
    /// The GS base.
    pub teb: u64,
    pub peb: u64,
    pub image_base: u64,
    /// The executable was moved from its preferred base.
    pub relocated: bool,
    pub stack_base: u64,
    pub stack_limit: u64,
}

/// Set up `proc` (new, empty) to run the PE executable `file` in its
/// first thread `thread`; `file_object` is the executable's File object,
/// if the kernel made one (the image section references it). `b` is the
/// new process's memory. On failure the caller ends the process with
/// [`process_exit`].
pub fn start_process(
    sys: &mut NtSystem,
    proc: &mut NtProcess,
    b: &mut dyn Backend,
    file: &[u8],
    file_object: Option<ObjectId>,
    thread: ThreadRef,
) -> Result<ProcessStart, NtStatus> {
    let om = &mut sys.objects;
    om.create_process(proc.pid, None)?;
    let pobj =
        NewObject::Opaque { kind: ObjectType::Process, cookie: proc.pid.0.into(), signaled: Some(false), parse: false };
    let pid_object = om.new_object(pobj, thread)?;
    proc.object = Some(pid_object);
    om.set_process_object(proc.pid, pid_object)?;

    let image = ImageSection::from_file(file)?;
    if image.info.directory(poly_pe::dir::IMPORT).is_some_and(|d| d.size != 0) {
        // Binding imports is our ntdll's job.
        return Err(STATUS_INVALID_IMAGE_FORMAT);
    }
    let (entry_rva, reserve, commit) = (
        u64::from(image.info.entry_point()),
        image.info.optional.size_of_stack_reserve,
        image.info.optional.size_of_stack_commit,
    );
    let source = SectionSource::Image(image.image.clone());
    let backing = b.create_section(&source)?;
    if let Some(f) = file_object {
        sys.objects.reference(f)?;
    }
    let protect = Protection::new(PAGE_EXECUTE_WRITECOPY, false).expect("valid");
    let size = image.size;
    let key = sys.add_section(Section {
        size,
        protect,
        attributes: SEC_IMAGE,
        backing,
        kind: SectionKind::Image { image: alloc::boxed::Box::new(image), file: file_object },
    });
    let new = NewObject::Opaque { kind: ObjectType::Section, cookie: key, signaled: None, parse: false };
    let section_object = match sys.objects.new_object(new, thread) {
        Ok(id) => id,
        Err(e) => {
            sys.remove_section(key, b);
            return Err(e);
        }
    };
    let req = MapView {
        section: Handle::NULL,
        base: 0,
        zero_bits: 0,
        commit_size: 0,
        offset: None,
        view_size: 0,
        inherit: VIEW_SHARE,
        allocation_type: 0,
        protect: PAGE_EXECUTE_WRITECOPY,
    };
    let mapped = section::map_section(sys, proc, b, section_object, &req, true);
    // The view holds its own reference; the section goes with the view.
    let _ = sys.objects.dereference(section_object);
    sys.reap(b);
    let mapped = mapped?;
    let image_base = mapped.base;
    let preferred = sys.section(key).and_then(Section::image).map_or(image_base, ImageSection::image_base);

    let rw = PAGE_READWRITE;
    let anywhere_top = MEM_RESERVE | MEM_COMMIT | MEM_TOP_DOWN;
    let (peb, _) = proc.vm.allocate(b, 0, PAGE_SIZE, 0, anywhere_top, rw)?;
    let (teb, _) = proc.vm.allocate(b, 0, TEB_SIZE, 0, anywhere_top, rw)?;

    let reserve = page_up(reserve.max(ALLOCATION_GRANULARITY)).ok_or(STATUS_INVALID_IMAGE_FORMAT)?;
    let commit = page_up(commit.max(PAGE_SIZE)).ok_or(STATUS_INVALID_IMAGE_FORMAT)?.min(reserve - 2 * PAGE_SIZE);
    let (stack, stack_size) = proc.vm.allocate(b, 0, reserve, 0, MEM_RESERVE, rw)?;
    let top = stack + stack_size;
    let limit = top - commit;
    proc.vm.allocate(b, limit, commit, 0, MEM_COMMIT, rw)?;
    proc.vm.allocate(b, limit - PAGE_SIZE, PAGE_SIZE, 0, MEM_COMMIT, rw | PAGE_GUARD)?;
    proc.vm.set_stack(stack);

    let mut tib = [0u8; 0x68];
    tib[TEB_STACK_BASE as usize..][..8].copy_from_slice(&top.to_le_bytes());
    tib[TEB_STACK_LIMIT as usize..][..8].copy_from_slice(&limit.to_le_bytes());
    tib[TEB_SELF as usize..][..8].copy_from_slice(&teb.to_le_bytes());
    tib[TEB_PEB as usize..][..8].copy_from_slice(&peb.to_le_bytes());
    b.write(teb, &tib)?;

    let tobj = NewObject::Opaque { kind: ObjectType::Thread, cookie: thread.0, signaled: Some(false), parse: false };
    let thread_object = sys.objects.new_object(tobj, thread)?;
    sys.objects.set_thread_object(thread, Some(thread_object))?;
    proc.threads.push(NtThread { thread, object: thread_object, teb, stack });
    proc.peb = peb;
    proc.image_base = image_base;

    Ok(ProcessStart {
        entry: image_base + entry_rva,
        rsp: top - 0x28,
        teb,
        peb,
        image_base,
        relocated: image_base != preferred,
        stack_base: top,
        stack_limit: limit,
    })
}

/// The process has ended (its last thread is going): drop its views'
/// section references, its threads' and its own objects, and close every
/// handle in its table. Its memory itself goes with the kernel's memory
/// map; `b` is only used to free section pages.
pub fn process_exit(sys: &mut NtSystem, proc: &mut NtProcess, b: &mut dyn Backend) {
    for a in proc.vm.clear() {
        if let Some(v) = a.view() {
            let _ = sys.objects.dereference(v.object);
        }
    }
    for t in core::mem::take(&mut proc.threads) {
        let _ = sys.objects.set_thread_object(t.thread, None);
        let _ = sys.objects.set_signaled(t.object, true);
        let _ = sys.objects.dereference(t.object);
    }
    let _ = sys.objects.destroy_process(proc.pid);
    if let Some(p) = proc.object.take() {
        let _ = sys.objects.set_signaled(p, true);
        let _ = sys.objects.dereference(p);
    }
    sys.reap(b);
}

/// A page fault at `addr` that the kernel's memory map could not resolve:
/// a guard page fires (and a stack grows: the thread's TEB gets its new
/// `StackLimit`), or it is an access violation.
pub fn access_fault(proc: &mut NtProcess, b: &mut dyn Backend, thread: ThreadRef, addr: u64) -> AccessFault {
    let fault = proc.vm.access_fault(b, addr);
    if let AccessFault::StackGrown { limit } = fault
        && let Some(t) = proc.thread(thread)
    {
        let _ = b.write(t.teb + TEB_STACK_LIMIT, &limit.to_le_bytes());
    }
    fault
}

/// The exception code a CPU exception in user mode raises (the
/// `EXCEPTION_*` values of `winbase.h` are these `ntstatus.h` codes; which
/// vector raises which is from the Intel SDM's exception list and Learn's
/// `EXCEPTION_RECORD` descriptions). A general protection fault (a
/// privileged instruction or a non-canonical address) is
/// `STATUS_ACCESS_VIOLATION` for now; to be confirmed by the spec agent.
pub fn exception_status(vector: u8) -> NtStatus {
    match vector {
        0 => NtStatus(0xC000_0094),       // STATUS_INTEGER_DIVIDE_BY_ZERO
        1 => NtStatus(0x8000_0004),       // STATUS_SINGLE_STEP
        3 => NtStatus(0x8000_0003),       // STATUS_BREAKPOINT
        4 => NtStatus(0xC000_0095),       // STATUS_INTEGER_OVERFLOW
        5 => NtStatus(0xC000_008C),       // STATUS_ARRAY_BOUNDS_EXCEEDED
        6 => NtStatus(0xC000_001D),       // STATUS_ILLEGAL_INSTRUCTION
        16 | 19 => NtStatus(0xC000_0090), // STATUS_FLOAT_INVALID_OPERATION
        17 => NtStatus(0x8000_0002),      // STATUS_DATATYPE_MISALIGNMENT
        _ => STATUS_ACCESS_VIOLATION,
    }
}
