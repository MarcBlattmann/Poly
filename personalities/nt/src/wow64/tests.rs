//! Host tests of the 32-bit system-call path (ADR 0186): the fake kernel plays a 32-bit process
//! whose arguments are 4-byte values in memory below 4 GiB.

extern crate std;

use super::*;
use crate::services::Service;
use crate::tests::{Fake, SELF};
use crate::vm::{LOWEST_ADDRESS, VirtualMemory};
use alloc::vec;
use alloc::vec::Vec;
use poly_abi::{Outcome, SyscallRegs};
use poly_ntobj::status::{
    STATUS_ACCESS_VIOLATION, STATUS_INFO_LENGTH_MISMATCH, STATUS_NOT_IMPLEMENTED, STATUS_SUCCESS,
};

/// Where the test's 32-bit caller keeps its arguments and data.
const LOW: u64 = 0x6000_0000;
const LOW_SIZE: u64 = 0x1_0000;

struct Caller32 {
    f: Fake,
    next: u64,
}

impl Caller32 {
    fn new() -> Caller32 {
        let mut f = Fake::new();
        f.proc.wow64 = true;
        f.proc.vm = VirtualMemory::with_range(LOWEST_ADDRESS, address_end(0));
        f.map_test_memory(LOW, LOW_SIZE);
        f.accept_contexts = true;
        Caller32 { f, next: LOW + 0x100 }
    }

    /// `data` in the caller's memory (8-aligned); its address.
    fn put(&mut self, data: &[u8]) -> u32 {
        let at = self.next;
        self.next += (data.len() as u64).div_ceil(8) * 8;
        self.f.mem.write_bytes(at, data).unwrap();
        at as u32
    }

    fn put32(&mut self, v: u32) -> u32 {
        self.put(&v.to_le_bytes())
    }

    fn u32_at(&self, addr: u32) -> u32 {
        self.f.u32_at(u64::from(addr))
    }

    /// A 32-bit `UNICODE_STRING` for `s`.
    fn unicode(&mut self, s: &str) -> u32 {
        let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let buf = self.put(&units);
        let mut h = [0u8; 8];
        h[0..2].copy_from_slice(&(units.len() as u16).to_le_bytes());
        h[2..4].copy_from_slice(&(units.len() as u16).to_le_bytes());
        h[4..8].copy_from_slice(&buf.to_le_bytes());
        self.put(&h)
    }

    /// A 32-bit `OBJECT_ATTRIBUTES` naming `s`.
    fn attrs(&mut self, s: &str, attributes: u32) -> u32 {
        let name = self.unicode(s);
        let mut oa = [0u8; 24];
        oa[0..4].copy_from_slice(&24u32.to_le_bytes());
        oa[8..12].copy_from_slice(&name.to_le_bytes());
        oa[12..16].copy_from_slice(&attributes.to_le_bytes());
        self.put(&oa)
    }

    /// Call `service` the way the i686 `ntdll`'s stub does: the arguments on the stack, EDX
    /// pointing at them.
    fn call_outcome(&mut self, number: u32, args: &[u32]) -> (Outcome, NtStatus) {
        let bytes: Vec<u8> = args.iter().flat_map(|a| a.to_le_bytes()).collect();
        let at = LOW;
        self.f.mem.write_bytes(at, &bytes).unwrap();
        let mut regs = SyscallRegs { rax: u64::from(number), rdx: at, ..Default::default() };
        let outcome = dispatch32(&mut self.f, &mut regs, |k, r| crate::dispatch_with(k, r));
        (outcome, NtStatus(regs.rax as u32))
    }

    fn call(&mut self, service: Service, args: &[u32]) -> NtStatus {
        let (outcome, status) = self.call_outcome(service.number(), args);
        assert_eq!(outcome, Outcome::Handled);
        status
    }
}

const CURRENT: u32 = 0xFFFF_FFFF;

#[test]
fn the_address_space_ends_at_two_or_four_gib() {
    assert_eq!(address_end(0x0102), 0x7FFF_0000);
    assert_eq!(address_end(0x0122), 0xFFFF_0000);
}

#[test]
fn every_service_has_one_signature_and_no_more_arguments_than_the_window_holds() {
    for &(service, _, name) in crate::services::all() {
        let rows = sigs::SIGNATURES.iter().filter(|(s, _)| *s == service).count();
        assert_eq!(rows, 1, "{name}");
        assert!(sigs::signature(service).unwrap().len() <= 14, "{name}");
    }
    assert_eq!(sigs::SIGNATURES.len(), crate::services::all().count());
}

#[test]
fn allocating_memory_widens_the_pseudo_handle_and_writes_back_four_byte_results() {
    let mut c = Caller32::new();
    let base = c.put32(0);
    let size = c.put32(0x3000);
    let status = c.call(Service::AllocateVirtualMemory, &[CURRENT, base, 0, size, 0x3000, 0x04]);
    assert_eq!(status, STATUS_SUCCESS);
    let (b, s) = (c.u32_at(base), c.u32_at(size));
    assert!(b != 0 && u64::from(b) < 0x7FFF_0000, "{b:#x}");
    assert_eq!(s, 0x3000);
    // The four bytes after each result are the caller's, untouched.
    assert_eq!(c.u32_at(base + 4), 0);
    // And it is real memory of the process.
    assert!(c.f.proc.vm.allocation(u64::from(b)).is_some());
    let _ = SELF;
}

#[test]
fn a_named_event_is_created_and_opened_through_32_bit_object_attributes() {
    let mut c = Caller32::new();
    let handle = c.put32(0xDEAD_BEEF);
    let oa = c.attrs("\\BaseNamedObjects\\wow64-event", 0);
    assert_eq!(c.call(Service::CreateEvent, &[handle, 0x1F_0003, oa, 0, 0]), STATUS_SUCCESS);
    let h = c.u32_at(handle);
    assert!(h != 0 && h != 0xDEAD_BEEF && h < 0x0100_0000, "{h:#x}");
    let opened = c.put32(0);
    let oa = c.attrs("\\BaseNamedObjects\\wow64-event", 0);
    assert_eq!(c.call(Service::OpenEvent, &[opened, 0x1F_0003, oa]), STATUS_SUCCESS);
    assert_ne!(c.u32_at(opened), 0);
    assert_eq!(c.call(Service::SetEvent, &[h, 0]), STATUS_SUCCESS);
    // A wait on it with a null timeout returns at once: it is signalled.
    assert_eq!(c.call(Service::WaitForSingleObject, &[c.u32_at(opened), 0, 0]), STATUS_SUCCESS);
    assert_eq!(c.call(Service::Close, &[h]), STATUS_SUCCESS);
}

#[test]
fn system_basic_information_has_the_32_bit_layout_and_address_limit() {
    let mut c = Caller32::new();
    let buf = c.put(&[0xAA; 64]);
    let ret = c.put32(0);
    assert_eq!(c.call(Service::QuerySystemInformation, &[0, buf, 44, ret]), STATUS_SUCCESS);
    assert_eq!(c.u32_at(ret), 44);
    // NumberOfProcessors at its 32-bit offset (56 in 64 bits), and the process's own range.
    let processors = c.f.bytes_at(u64::from(buf) + 40, 1)[0];
    assert!(processors >= 1);
    assert_eq!(c.u32_at(buf + 28), 0x1_0000, "MinimumUserModeAddress");
    assert_eq!(c.u32_at(buf + 32), 0x7FFE_FFFF, "MaximumUserModeAddress");
    // Bytes past the 44 are the caller's.
    assert_eq!(c.f.bytes_at(u64::from(buf) + 44, 4), vec![0xAA; 4]);
    // Too small a buffer: the 32-bit size is the one reported.
    assert_eq!(c.call(Service::QuerySystemInformation, &[0, buf, 40, ret]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(c.u32_at(ret), 44);
}

#[test]
fn memory_basic_information_has_the_32_bit_layout() {
    let mut c = Caller32::new();
    let base = c.put32(0);
    let size = c.put32(0x2000);
    assert_eq!(c.call(Service::AllocateVirtualMemory, &[CURRENT, base, 0, size, 0x3000, 0x04]), STATUS_SUCCESS);
    let at = c.u32_at(base);
    let mbi = c.put(&[0u8; 28]);
    let ret = c.put32(0);
    assert_eq!(c.call(Service::QueryVirtualMemory, &[CURRENT, at, 0, mbi, 28, ret]), STATUS_SUCCESS);
    assert_eq!(c.u32_at(ret), 28);
    assert_eq!(c.u32_at(mbi), at, "BaseAddress");
    assert_eq!(c.u32_at(mbi + 4), at, "AllocationBase");
    assert_eq!(c.u32_at(mbi + 8), 0x04, "AllocationProtect");
    assert_eq!(c.u32_at(mbi + 12), 0x2000, "RegionSize");
    assert_eq!(c.u32_at(mbi + 16), 0x1000, "State: MEM_COMMIT");
    assert_eq!(c.u32_at(mbi + 20), 0x04, "Protect");
    assert_eq!(c.u32_at(mbi + 24), 0x2_0000, "Type: MEM_PRIVATE");
}

#[test]
fn an_unconverted_structure_and_an_unknown_service_are_not_implemented() {
    let mut c = Caller32::new();
    let ctx = c.put(&[0u8; 0x2CC]);
    assert_eq!(c.call(Service::SetContextThread, &[0xFFFF_FFFE, ctx]), STATUS_NOT_IMPLEMENTED);
    // A win32k (table 1) number has no 32-bit signature yet.
    let (outcome, status) = c.call_outcome(0x1000, &[]);
    assert!(matches!(outcome, Outcome::Unimplemented { .. }));
    assert_eq!(status, STATUS_NOT_IMPLEMENTED);
}

#[test]
fn a_bad_argument_pointer_is_an_access_violation() {
    let mut c = Caller32::new();
    let mut regs = SyscallRegs { rax: u64::from(Service::Close.number()), rdx: 0x10, ..Default::default() };
    let _ = dispatch32(&mut c.f, &mut regs, |k, r| crate::dispatch_with(k, r));
    assert_eq!(NtStatus(regs.rax as u32), STATUS_ACCESS_VIOLATION);
}

#[test]
fn the_start_frame_holds_the_routine_its_argument_and_the_image_base() {
    let mut c = Caller32::new();
    let top = LOW + LOW_SIZE;
    let esp = start_esp(&mut c.f.mem, top, [0x0040_1000, 0x7FFD_0000, 0x0040_0000]).unwrap();
    assert_eq!(esp, top - 16);
    let words: Vec<u32> = (0..4).map(|i| c.f.u32_at(esp + 4 * i)).collect();
    assert_eq!(words, [0, 0x0040_1000, 0x7FFD_0000, 0x0040_0000]);
}

#[test]
fn system32_is_redirected_to_syswow64_except_the_exempt_directories() {
    let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let r = |s: &str| redirect_system32(&w(s)).map(|v| std::string::String::from_utf16(&v).unwrap());
    assert_eq!(r("C:\\Windows\\System32\\kernel32.dll").as_deref(), Some("C:\\Windows\\SysWOW64\\kernel32.dll"));
    assert_eq!(r("\\??\\c:\\windows\\system32\\x\\y.dll").as_deref(), Some("\\??\\c:\\Windows\\SysWOW64\\x\\y.dll"));
    assert_eq!(r("C:\\Windows\\System32").as_deref(), Some("C:\\Windows\\SysWOW64"));
    assert_eq!(r("C:\\Windows\\System32\\drivers\\etc\\hosts"), None);
    assert_eq!(r("C:\\Windows\\System32\\spool\\x"), None);
    assert_eq!(r("C:\\Windows\\System32x\\a"), None);
    assert_eq!(r("C:\\Windows\\SysWOW64\\a"), None);
    assert_eq!(r("D:\\Windows\\System32\\a"), None);
}

#[test]
fn the_process_says_it_is_a_32_bit_one() {
    let mut c = Caller32::new();
    let value = c.put32(0);
    let ret = c.put32(0);
    assert_eq!(c.call(Service::QueryInformationProcess, &[CURRENT, 26, value, 4, ret]), STATUS_SUCCESS);
    assert_eq!(c.u32_at(ret), 4);
    assert_eq!(c.u32_at(value), c.f.proc.peb as u32);
    // A 64-bit process asking about itself: zero.
    let mut f = Fake::new();
    let out = f.alloc(8);
    assert_eq!(f.call(Service::QueryInformationProcess, &[SELF, 26, out, 8, 0]), STATUS_SUCCESS);
    assert_eq!(f.u64_at(out), 0);
}

/// A stand-in for the i686 `ntdll`: a PE32 DLL exporting the three entry points.
fn fake_ntdll32() -> &'static [u8] {
    use poly_ldr::builder::{Export, PeBuilder, Section, TEXT, Target};
    let export = |name: &str, ordinal, rva| Export { name: Some(name.into()), ordinal, target: Target::Rva(rva) };
    let dll = PeBuilder {
        image_base: 0x7700_0000,
        dll: true,
        sections: vec![Section::new(".text", vec![0xCC; 0x30], TEXT)],
        export_name: "ntdll.dll".into(),
        exports: vec![
            export("LdrInitializeThunk", 1, 0x1000),
            export("KiUserApcDispatcher", 2, 0x1010),
            export("KiUserExceptionDispatcher", 3, 0x1020),
        ],
        ..PeBuilder::x86()
    }
    .write();
    alloc::boxed::Box::leak(dll.into_boxed_slice())
}

/// A PE32 executable at 0x400000 with its entry at RVA 0x1000.
fn exe32() -> Vec<u8> {
    use poly_ldr::builder::{PeBuilder, Section, TEXT};
    PeBuilder {
        image_base: 0x40_0000,
        entry: 0x1000,
        sections: vec![Section::new(".text", vec![0xC3], TEXT)],
        ..PeBuilder::x86()
    }
    .write()
}

#[test]
fn a_pe32_executable_starts_a_32_bit_process_below_4_gib() {
    let mut f = Fake::bare();
    f.sys.ntdll32 = fake_ntdll32();
    let tid = crate::tests::TID;
    let ps = crate::exec::start_process(
        &mut f.sys,
        &mut f.proc,
        &mut f.mem,
        &exe32(),
        &crate::tests::fake_ntdll(),
        None,
        tid,
    )
    .expect("starts");
    assert!(ps.wow64 && f.proc.wow64);
    // Large-address aware (the builder marks it so): a 4 GiB space.
    assert_eq!(f.proc.vm.end(), 0xFFFF_0000);
    assert_eq!(ps.image_base, 0x40_0000);
    assert_eq!(ps.ntdll_base, 0x7700_0000);
    assert_eq!(ps.entry, 0x7700_1000, "LdrInitializeThunk of the i686 ntdll");
    for a in [ps.teb, ps.peb, ps.rsp, ps.stack_base] {
        assert!(a < 1 << 32, "{a:#x}");
    }
    let u32_at = |f: &Fake, a: u64| f.u32_at(a);
    let l = poly_rtl::layout::X86;
    // The 32-bit TEB: an empty SEH chain, the stack, itself, the client id, the PEB.
    assert_eq!(u32_at(&f, ps.teb + l.teb_exception_list), 0xFFFF_FFFF);
    assert_eq!(u64::from(u32_at(&f, ps.teb + l.teb_stack_base)), ps.stack_base);
    assert_eq!(u64::from(u32_at(&f, ps.teb + l.teb_stack_limit)), ps.stack_limit);
    assert_eq!(u64::from(u32_at(&f, ps.teb + l.teb_self)), ps.teb);
    assert_eq!(u64::from(u32_at(&f, ps.teb + l.teb_client_id + 4)), tid.0);
    assert_eq!(u64::from(u32_at(&f, ps.teb + l.teb_peb)), ps.peb);
    assert_eq!(u32_at(&f, ps.peb + l.peb_image_base), 0x40_0000);
    // The start frame: a zero return address, the entry point, the PEB, the image base.
    let frame: Vec<u32> = (0..4).map(|i| u32_at(&f, ps.rsp + 4 * i)).collect();
    assert_eq!(frame, [0, 0x40_1000, ps.peb as u32, 0x40_0000]);
    // Exceptions go to the i686 ntdll's dispatcher; APCs are not delivered to 32-bit code yet.
    assert_eq!((f.proc.exception_dispatcher, f.proc.apc_dispatcher), (0x7700_1020, 0));
    // The parameters have the 32-bit layout.
    let params = crate::params::for_program("C:\\Apps\\app.exe", &["a b"], vec![0, 0]);
    let base = crate::params::write(&mut f.proc, &mut f.mem, ps.peb, &params).unwrap();
    assert!(base < 1 << 32);
    assert_eq!(u64::from(u32_at(&f, ps.peb + l.peb_process_parameters)), base);
    let len = f.bytes_at(base + l.pp_image_path_name, 2);
    assert_eq!(u16::from_le_bytes([len[0], len[1]]) as usize, "C:\\Apps\\app.exe".len() * 2);
    let buffer = u64::from(u32_at(&f, base + l.pp_image_path_name + 4));
    let text: Vec<u16> =
        f.bytes_at(buffer, "C:\\Apps\\app.exe".len() * 2).chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(std::string::String::from_utf16(&text).unwrap(), "C:\\Apps\\app.exe");
}

#[test]
fn without_an_i686_ntdll_a_pe32_executable_is_refused() {
    let mut f = Fake::bare();
    let r = crate::exec::start_process(
        &mut f.sys,
        &mut f.proc,
        &mut f.mem,
        &exe32(),
        &crate::tests::fake_ntdll(),
        None,
        crate::tests::TID,
    );
    assert_eq!(r.err(), Some(poly_ntobj::status::STATUS_INVALID_IMAGE_FORMAT));
}

/// The name a file service's converted `OBJECT_ATTRIBUTES` carries (its first argument).
fn converted_name(c: &mut Caller32, service: Service, oa: u32, rest: &[u32]) -> std::string::String {
    let mut args = vec![oa];
    args.extend_from_slice(rest);
    let bytes: Vec<u8> = args.iter().flat_map(|a| a.to_le_bytes()).collect();
    c.f.mem.write_bytes(LOW, &bytes).unwrap();
    let mut regs = SyscallRegs { rax: u64::from(service.number()), rdx: LOW, ..Default::default() };
    let mut seen = None;
    dispatch32(&mut c.f, &mut regs, |k, r| {
        seen = crate::abi::read_attributes(k, r.r10).ok().and_then(|a| a.name);
        Outcome::Handled
    });
    std::string::String::from_utf16(&seen.expect("a name")).unwrap()
}

#[test]
fn a_32_bit_process_opening_system32_gets_syswow64() {
    let mut c = Caller32::new();
    let oa = c.attrs("\\??\\C:\\Windows\\System32\\kernel32.dll", 0x40);
    let info = c.put(&[0u8; 40]);
    assert_eq!(
        converted_name(&mut c, Service::QueryAttributesFile, oa, &[info]),
        "\\??\\C:\\Windows\\SysWOW64\\kernel32.dll"
    );
    // The exempt directories keep their names.
    let oa = c.attrs("\\??\\C:\\Windows\\System32\\drivers\\etc\\hosts", 0x40);
    assert_eq!(
        converted_name(&mut c, Service::QueryAttributesFile, oa, &[info]),
        "\\??\\C:\\Windows\\System32\\drivers\\etc\\hosts"
    );
}

#[test]
fn lastgood_and_regedit_are_redirected_too() {
    let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let r = |s: &str| redirect_system32(&w(s)).map(|v| std::string::String::from_utf16(&v).unwrap());
    // Learn's table: %windir%\lastgood\system32 and %windir%\regedit.exe.
    assert_eq!(r("C:\\Windows\\lastgood\\system32\\a.dll").as_deref(), Some("C:\\Windows\\lastgood\\SysWOW64\\a.dll"));
    assert_eq!(r("c:\\WINDOWS\\LastGood\\System32").as_deref(), Some("c:\\WINDOWS\\LastGood\\SysWOW64"));
    assert_eq!(r("C:\\Windows\\regedit.exe").as_deref(), Some("C:\\Windows\\SysWOW64\\regedit.exe"));
    assert_eq!(r("C:\\Windows\\regedit.exe.bak"), None);
    assert_eq!(r("C:\\Windows\\lastgood\\other"), None);
}

#[test]
fn sysnative_names_the_real_system32_and_is_not_a_directory() {
    let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let n = |s: &str| sysnative_to_system32(&w(s)).map(|v| std::string::String::from_utf16(&v).unwrap());
    assert_eq!(n("C:\\Windows\\Sysnative\\notepad.exe").as_deref(), Some("C:\\Windows\\System32\\notepad.exe"));
    assert_eq!(n("\\??\\c:\\windows\\SYSNATIVE").as_deref(), Some("\\??\\c:\\Windows\\System32"));
    assert_eq!(n("C:\\Windows\\Sysnativex\\a"), None);
    assert_eq!(n("C:\\Windows\\System32\\a"), None);
    // `file_view` maps it with redirection on and off, and System32 only with it on.
    let v = |s: &str, off: bool| file_view(&w(s), off).map(|v| std::string::String::from_utf16(&v).unwrap());
    assert_eq!(v("C:\\Windows\\Sysnative\\a", false).as_deref(), Some("C:\\Windows\\System32\\a"));
    assert_eq!(v("C:\\Windows\\Sysnative\\a", true).as_deref(), Some("C:\\Windows\\System32\\a"));
    assert_eq!(v("C:\\Windows\\System32\\a", false).as_deref(), Some("C:\\Windows\\SysWOW64\\a"));
    assert_eq!(v("C:\\Windows\\System32\\a", true), None);
}

#[test]
fn a_thread_that_switched_redirection_off_reaches_the_real_system32() {
    let mut c = Caller32::new();
    let info = c.put(&[0u8; 40]);
    let teb = c.f.proc.threads[0].teb;
    let flag = teb + TEB_FS_REDIRECTION_OFF;
    let system32 = "\\??\\C:\\Windows\\System32\\kernel32.dll";
    // Off (the flag's value 1): the name stays, and Sysnative still maps to System32.
    c.f.mem.write_bytes(flag, &1u32.to_le_bytes()).unwrap();
    let oa = c.attrs(system32, 0x40);
    assert_eq!(converted_name(&mut c, Service::QueryAttributesFile, oa, &[info]), system32);
    let oa = c.attrs("\\??\\C:\\Windows\\Sysnative\\kernel32.dll", 0x40);
    assert_eq!(converted_name(&mut c, Service::QueryAttributesFile, oa, &[info]), system32);
    // Back on (zero, as in a new thread): redirected again.
    c.f.mem.write_bytes(flag, &0u32.to_le_bytes()).unwrap();
    let oa = c.attrs(system32, 0x40);
    assert_eq!(
        converted_name(&mut c, Service::QueryAttributesFile, oa, &[info]),
        "\\??\\C:\\Windows\\SysWOW64\\kernel32.dll"
    );
    let oa = c.attrs("\\??\\C:\\Windows\\Sysnative\\kernel32.dll", 0x40);
    assert_eq!(converted_name(&mut c, Service::QueryAttributesFile, oa, &[info]), system32);
}

#[test]
fn a_raised_exception_reaches_the_32_bit_dispatcher_with_x86_structures() {
    use crate::wow64::context::{CONTEXT32_SIZE, from_context32, to_record32};
    let mut c = Caller32::new();
    c.f.proc.exception_dispatcher = 0x7700_1020;
    // The thread's stack: a page of the test memory.
    let esp = (LOW + LOW_SIZE - 0x100) as u32;
    let mut ctx = [0u8; CONTEXT32_SIZE];
    ctx[0..4].copy_from_slice(&0x1_0007u32.to_le_bytes());
    ctx[0xB8..0xBC].copy_from_slice(&0x40_1234u32.to_le_bytes());
    ctx[0xC4..0xC8].copy_from_slice(&esp.to_le_bytes());
    ctx[0xB0..0xB4].copy_from_slice(&0xAAAAu32.to_le_bytes());
    let ctx_at = c.put(&ctx);
    let rec = poly_seh::ExceptionRecord::new(0xE000_0001, 0, 0x40_1234, &[7]);
    let rec_at = c.put(&to_record32(&rec));
    assert_eq!(c.call(Service::RaiseException, &[rec_at, ctx_at, 1]), STATUS_SUCCESS);
    // The fake kernel recorded the dispatcher's registers.
    let loaded = c.f.loaded.clone().expect("a context was loaded");
    assert_eq!(loaded.rip, 0x7700_1020);
    let sp = loaded.rsp();
    assert!(sp < u64::from(esp) && sp >= LOW);
    let words: Vec<u32> = (0..3).map(|i| c.f.u32_at(sp + 4 * i)).collect();
    assert_eq!(words[0], 0, "the zero return address");
    let (record32, context32) = (u64::from(words[1]), u64::from(words[2]));
    assert_eq!(c.f.u32_at(record32), 0xE000_0001);
    assert_eq!(c.f.u32_at(record32 + 16), 1);
    assert_eq!(c.f.u32_at(record32 + 20), 7);
    let back = from_context32(&c.f.bytes_at(context32, CONTEXT32_SIZE));
    assert_eq!((back.rip, back.gpr[0], back.gpr[4]), (0x40_1234, 0xAAAA, u64::from(esp)));
}

#[test]
fn nt_continue_loads_an_x86_context() {
    let mut c = Caller32::new();
    let mut ctx = [0u8; 0x2CC];
    ctx[0..4].copy_from_slice(&0x1_0003u32.to_le_bytes());
    ctx[0xB8..0xBC].copy_from_slice(&0x40_2000u32.to_le_bytes());
    ctx[0xC4..0xC8].copy_from_slice(&0x0012_F000u32.to_le_bytes());
    ctx[0xA8..0xAC].copy_from_slice(&0x55u32.to_le_bytes());
    let at = c.put(&ctx);
    assert_eq!(c.call(Service::Continue, &[at, 0]), STATUS_SUCCESS);
    let loaded = c.f.loaded.clone().expect("loaded");
    assert_eq!((loaded.rip, loaded.rsp(), loaded.gpr[2]), (0x40_2000, 0x12_F000, 0x55));
}
