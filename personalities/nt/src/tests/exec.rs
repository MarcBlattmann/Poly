use super::*;
use crate::consts::*;
use crate::exec::{self, TEB_PEB, TEB_SELF, TEB_STACK_BASE, TEB_STACK_LIMIT};
use crate::vm::{AccessFault, STATUS_STACK_OVERFLOW};
use poly_ntobj::ObjectType;
use poly_ntobj::status::*;

fn start(f: &mut Fake, file: &[u8]) -> core::result::Result<exec::ProcessStart, NtStatus> {
    exec::start_process(&mut f.sys, &mut f.proc, &mut f.mem, file, &fake_ntdll(), None, TID)
}

#[test]
fn image_is_mapped_at_its_base_with_section_protections() {
    let mut f = Fake::bare();
    let s = start(&mut f, &test_exe(false)).unwrap();
    assert_eq!(s.image_base, IMAGE_BASE);
    assert!(!s.relocated);
    // The thread starts in ntdll, which calls the image entry.
    assert_eq!(s.start_routine, IMAGE_BASE + 0x1000);
    assert_eq!(s.argument, s.peb);
    assert_eq!(s.entry, NTDLL_BASE + u64::from(NTDLL_THREAD_START));
    assert_eq!(f.proc.vm.query(s.entry).unwrap().protect, PAGE_EXECUTE_READ);
    let q = |f: &Fake, a| f.proc.vm.query(a).unwrap();
    let headers = q(&f, IMAGE_BASE);
    assert_eq!((headers.state, headers.type_, headers.protect), (MEM_COMMIT, MEM_IMAGE, PAGE_READONLY));
    assert_eq!(headers.allocation_base, IMAGE_BASE);
    assert_eq!(headers.region_size, 0x1000);
    assert_eq!(q(&f, IMAGE_BASE + 0x1000).protect, PAGE_EXECUTE_READ);
    let data = q(&f, IMAGE_BASE + 0x2000);
    assert_eq!((data.protect, data.region_size), (PAGE_WRITECOPY, 0x2000));
    // The hardware sees the same: code runs, data is a private copy.
    assert_eq!(f.mem.rights(IMAGE_BASE + 0x1000), Some(HwProt::RX));
    assert_eq!(f.mem.rights(IMAGE_BASE + 0x2000), Some(HwProt::RW));
    assert_eq!(f.mem.bytes_at(IMAGE_BASE + 0x1000, 1), [0xC3]);
    assert_eq!(f.u64_at(IMAGE_BASE + 0x2008), IMAGE_BASE + 0x1000);
}

#[test]
fn teb_peb_and_stack() {
    let mut f = Fake::bare();
    let s = start(&mut f, &test_exe(false)).unwrap();
    assert_eq!(f.u64_at(s.teb + TEB_SELF), s.teb);
    assert_eq!(f.u64_at(s.teb + TEB_PEB), s.peb);
    assert_eq!(f.u64_at(s.teb + TEB_STACK_BASE), s.stack_base);
    assert_eq!(f.u64_at(s.teb + TEB_STACK_LIMIT), s.stack_limit);
    assert_eq!(f.bytes_at(s.peb + exec::PEB_BEING_DEBUGGED, 1), [0]);
    // The ClientId is the process and the first thread (its id is the pid).
    assert_eq!(f.u64_at(s.teb + exec::TEB_CLIENT_ID), u64::from(PID.0));
    assert_eq!(f.u64_at(s.teb + exec::TEB_CLIENT_ID + 8), TID.0);
    // The thread starts 8 bytes below the (16-aligned) stack top, as after
    // a call, at a zero return address that ends a stack walk.
    assert_eq!(s.rsp, s.stack_base - 8);
    assert_eq!(s.stack_base % 16, 0);
    assert_eq!(f.u64_at(s.rsp), 0);
    // 256 KiB reserved, 8 KiB committed, a guard page below.
    let stack = f.proc.vm.query(s.stack_limit).unwrap();
    assert_eq!((stack.state, stack.protect, stack.region_size), (MEM_COMMIT, PAGE_READWRITE, 0x2000));
    assert_eq!(stack.allocation_base, s.stack_base - 0x4_0000);
    let guard = f.proc.vm.query(s.stack_limit - 0x1000).unwrap();
    assert_eq!(guard.protect, PAGE_READWRITE | PAGE_GUARD);
    let reserved = f.proc.vm.query(stack.allocation_base).unwrap();
    assert_eq!((reserved.state, reserved.region_size), (MEM_RESERVE, 0x4_0000 - 0x3000));
    // TEB and PEB are top down, above the image and the stack.
    assert!(s.teb > s.stack_base && s.peb > s.teb);
    assert_eq!(f.proc.vm.query(s.teb).unwrap().region_size, exec::TEB_SIZE);
}

#[test]
fn a_second_threads_stack_grows_too() {
    let mut f = Fake::bare();
    start(&mut f, &test_exe(false)).unwrap();
    let tid = ThreadRef(TID.0 + 1);
    let req = crate::kernel::ThreadCreate {
        start_routine: 0x1000,
        argument: 0,
        stack_reserve: 0,
        stack_commit: 0,
        suspended: false,
        desired_access: 0x1F_FFFF,
    };
    let t = exec::create_thread(&mut f.sys, &mut f.proc, &mut f.mem, &req, tid).unwrap();
    let guard = t.stack_limit - 0x1000;
    let fault = exec::access_fault(&mut f.proc, &mut f.mem, tid, guard + 8);
    assert_eq!(fault, AccessFault::StackGrown { limit: guard });
    assert_eq!(f.u64_at(t.teb + TEB_STACK_LIMIT), guard);
}

#[test]
fn the_stack_grows_through_its_guard_page_until_the_last_page() {
    let mut f = Fake::bare();
    let s = start(&mut f, &test_exe(false)).unwrap();
    let guard = s.stack_limit - 0x1000;
    assert!(f.mem.read(guard, &mut [0u8; 1]).is_err());
    let fault = exec::access_fault(&mut f.proc, &mut f.mem, TID, guard + 8);
    assert_eq!(fault, AccessFault::StackGrown { limit: guard });
    assert_eq!(f.u64_at(s.teb + TEB_STACK_LIMIT), guard);
    assert!(f.mem.read(guard, &mut [0u8; 1]).is_ok());
    assert_eq!(f.proc.vm.query(guard - 0x1000).unwrap().protect, PAGE_READWRITE | PAGE_GUARD);
    // Grow to the end: the reservation's last page is never committed.
    let bottom = s.stack_base - 0x4_0000;
    let mut at = guard - 0x1000;
    loop {
        match exec::access_fault(&mut f.proc, &mut f.mem, TID, at) {
            AccessFault::StackGrown { limit } => assert_eq!(limit, at),
            AccessFault::Exception(e) => {
                assert_eq!(e, STATUS_STACK_OVERFLOW);
                break;
            }
        }
        at -= 0x1000;
    }
    assert_eq!(at, bottom + 0x1000);
    assert_eq!(f.proc.vm.query(bottom).unwrap().state, MEM_RESERVE);
    // Below the stack, nothing.
    assert_eq!(
        exec::access_fault(&mut f.proc, &mut f.mem, TID, bottom),
        AccessFault::Exception(STATUS_ACCESS_VIOLATION)
    );
}

#[test]
fn a_new_threads_stack_grows_through_its_guard_page_too() {
    // 7-Zip's worker threads (`_beginthreadex` with a 1 MiB reservation)
    // touch their guard page at once; that must grow the stack, as for the
    // first thread, not raise STATUS_GUARD_PAGE_VIOLATION (ADR 0091).
    let mut f = Fake::bare();
    start(&mut f, &test_exe(false)).unwrap();
    let req = crate::kernel::ThreadCreate {
        start_routine: IMAGE_BASE + 0x1000,
        argument: 0,
        stack_reserve: 0x10_0000,
        stack_commit: 0,
        suspended: false,
        desired_access: 0x1F_FFFF,
    };
    let t = ThreadRef(TID.0 + 1);
    let s = exec::create_thread(&mut f.sys, &mut f.proc, &mut f.mem, &req, t).unwrap();
    let guard = s.stack_limit - 0x1000;
    assert_eq!(f.proc.vm.query(guard).unwrap().protect, PAGE_READWRITE | PAGE_GUARD);
    let fault = exec::access_fault(&mut f.proc, &mut f.mem, t, guard + 0xFF8);
    assert_eq!(fault, AccessFault::StackGrown { limit: guard });
    assert_eq!(f.u64_at(s.teb + TEB_STACK_LIMIT), guard);
    assert_eq!(f.proc.vm.query(guard - 0x1000).unwrap().protect, PAGE_READWRITE | PAGE_GUARD);
    // Its reservation is the one asked for.
    assert_eq!(f.proc.vm.query(guard).unwrap().allocation_base, s.stack_base - 0x10_0000);
}

#[test]
fn an_image_whose_base_is_taken_is_relocated() {
    let mut f = Fake::bare();
    // Something already lives at the preferred base.
    f.proc.vm.allocate(&mut f.mem, IMAGE_BASE, 0x1000, 0, MEM_RESERVE, PAGE_READWRITE).unwrap();
    let s = start(&mut f, &test_exe(false)).unwrap();
    assert!(s.relocated);
    assert_ne!(s.image_base, IMAGE_BASE);
    assert_eq!(s.image_base % 0x1_0000, 0);
    assert_eq!(s.start_routine, s.image_base + 0x1000);
    assert_eq!(f.u64_at(s.image_base + 0x2008), s.image_base + 0x1000);
    // The headers record the new base.
    assert_eq!(f.u64_at(s.image_base + 0x40 + 24 + 24), s.image_base);
}

#[test]
fn a_fixed_image_whose_base_is_taken_fails() {
    let mut f = Fake::bare();
    f.proc.vm.allocate(&mut f.mem, IMAGE_BASE, 0x1000, 0, MEM_RESERVE, PAGE_READWRITE).unwrap();
    assert_eq!(start(&mut f, &test_exe(true)), Err(STATUS_CONFLICTING_ADDRESSES));
}

#[test]
fn not_an_image() {
    let mut f = Fake::bare();
    assert_eq!(start(&mut f, b"MZ nothing here"), Err(STATUS_INVALID_IMAGE_FORMAT));
}

#[test]
fn process_and_thread_objects_and_exit() {
    let mut f = Fake::new();
    // The executable's and ntdll's image sections.
    let before_sections = f.mem.live_sections();
    assert_eq!(before_sections, 2);
    let p = f.proc.object.unwrap();
    assert_eq!(f.sys.objects.object_type(p), Some(ObjectType::Process));
    assert_eq!(f.sys.objects.cookie(p), Some(u64::from(PID.0)));
    let empty = NtSystem::new().objects.object_count();
    exec::process_exit(&mut f.sys, &mut f.proc, &mut f.mem);
    assert!(!f.sys.objects.is_alive(p));
    assert_eq!(f.sys.section_count(), 0);
    assert_eq!(f.mem.live_sections(), 0);
    assert_eq!(f.sys.objects.object_count(), empty);
    assert_eq!(f.sys.objects.process_handle_count(PID), None);
}

#[test]
fn exceptions_map_to_status_codes() {
    assert_eq!(exec::exception_status(0), NtStatus(0xC000_0094));
    assert_eq!(exec::exception_status(3), NtStatus(0x8000_0003));
    assert_eq!(exec::exception_status(6), NtStatus(0xC000_001D));
    assert_eq!(exec::exception_status(14), STATUS_ACCESS_VIOLATION);
}

#[test]
fn terminate_and_trace() {
    let mut f = Fake::new();
    assert_eq!(f.call(Service::PolyTrace, &[1, 2, 3]), STATUS_SUCCESS);
    assert_eq!(f.traces, [[1, 2, 3]]);
    assert_eq!(f.call(Service::TerminateProcess, &[0, 5]), STATUS_SUCCESS);
    assert_eq!(f.terminated, None);
    assert_eq!(f.call(Service::TerminateProcess, &[SELF, 0x1234]), STATUS_SUCCESS);
    assert_eq!(f.terminated, Some(NtStatus(0x1234)));
    assert_eq!(f.call(Service::TerminateProcess, &[0x9998, 1]), STATUS_INVALID_HANDLE);
}

#[test]
fn unknown_services_are_not_implemented() {
    let mut f = Fake::new();
    let mut regs = SyscallRegs { rax: 0x7ff, ..Default::default() };
    assert_eq!(
        crate::dispatch_with(&mut f, &mut regs),
        poly_abi::Outcome::Unimplemented { table: "nt", number: 0x7ff }
    );
    assert_eq!(regs.rax, 0xC000_0002);
}

#[test]
fn ntdll_is_mapped_and_entered() {
    let mut f = Fake::bare();
    let s = start(&mut f, &test_exe(false)).unwrap();
    assert_eq!(s.ntdll_base, NTDLL_BASE);
    let headers = f.proc.vm.query(NTDLL_BASE).unwrap();
    assert_eq!((headers.state, headers.type_, headers.protect), (MEM_COMMIT, MEM_IMAGE, PAGE_READONLY));
    assert_eq!(f.proc.vm.query(NTDLL_BASE + 0x1000).unwrap().protect, PAGE_EXECUTE_READ);
    // Threads, APCs and exceptions enter its exports.
    assert_eq!(f.proc.thread_start, NTDLL_BASE + u64::from(NTDLL_THREAD_START));
    assert_eq!(f.proc.apc_dispatcher, NTDLL_BASE + u64::from(NTDLL_APC));
    assert_eq!(f.proc.exception_dispatcher, NTDLL_BASE + u64::from(NTDLL_EXCEPTION));
    assert_eq!(f.proc.ntdll_base, NTDLL_BASE);
}

#[test]
fn ntdll_whose_base_is_taken_is_relocated() {
    let mut f = Fake::bare();
    f.proc.vm.allocate(&mut f.mem, NTDLL_BASE, 0x1000, 0, MEM_RESERVE, PAGE_READWRITE).unwrap();
    let s = start(&mut f, &test_exe(false)).unwrap();
    assert_ne!(s.ntdll_base, NTDLL_BASE);
    assert_eq!(s.entry, s.ntdll_base + u64::from(NTDLL_THREAD_START));
    assert_eq!(f.proc.exception_dispatcher, s.ntdll_base + u64::from(NTDLL_EXCEPTION));
}

#[test]
fn a_dll_without_the_entry_points_is_not_our_ntdll() {
    use poly_ldr::builder::{PeBuilder, Section, TEXT};
    let mut f = Fake::bare();
    let other = PeBuilder { sections: vec![Section::new(".text", vec![0xC3], TEXT)], ..PeBuilder::default() }.write();
    let r = exec::start_process(&mut f.sys, &mut f.proc, &mut f.mem, &test_exe(false), &other, None, TID);
    assert_eq!(r, Err(STATUS_INVALID_IMAGE_FORMAT));
}

#[test]
fn an_executable_with_imports_starts_ntdll_binds_them() {
    use poly_ldr::builder::{Import, PeBuilder, Section, Sym, TEXT};
    let exe = PeBuilder {
        image_base: IMAGE_BASE,
        dll: false,
        entry: 0x1000,
        sections: vec![Section::new(".text", vec![0xC3], TEXT)],
        imports: vec![Import { dll: "ntdll.dll".into(), symbols: vec![Sym::name("NtClose")], ..Import::default() }],
        ..PeBuilder::default()
    }
    .write();
    let mut f = Fake::bare();
    let s = start(&mut f, &exe).unwrap();
    assert_eq!(s.start_routine, IMAGE_BASE + 0x1000);
}
