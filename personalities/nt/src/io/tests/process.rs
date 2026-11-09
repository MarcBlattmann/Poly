//! Tests: processes and file mappings over NT file I/O (ADR 0079).

use super::*;

// ----- processes and file mappings over NT file I/O (ADR 0079) -----------------

/// Parameters for `exe` built in the fake's scratch memory, as
/// `kernelbase` builds them for `NtCreateUserProcess`.
fn parameters_in(t: &mut T, exe: &str, cmd: &str, cwd: &str) -> (u64, crate::params::Parameters) {
    let p = crate::params::Parameters {
        image_path: exe.encode_utf16().collect(),
        command_line: cmd.encode_utf16().collect(),
        environment: "A=1\0POLY=child\0\0".encode_utf16().collect(),
        current_dir: cwd.encode_utf16().collect(),
        ..crate::params::Parameters::default()
    };
    let size = crate::params::image(0, &p).unwrap().len();
    let at = t.f.alloc(size);
    let bytes = crate::params::image(at, &p).unwrap();
    t.f.mem.write_bytes(at, &bytes).unwrap();
    (at, p)
}

fn create_process(t: &mut T, params: u64, thread_flags: u32) -> (NtStatus, u64, u64) {
    let (ph, th) = (t.f.alloc(8), t.f.alloc(8));
    let s = t.f.call(Service::CreateUserProcess, &[ph, th, 0, 0, 0, 0, 0, u64::from(thread_flags), params, 0, 0]);
    (s, t.f.u64_at(ph), t.f.u64_at(th))
}

fn exit_status_of(t: &mut T, h: u64) -> (NtStatus, u32, u64) {
    let buf = t.f.alloc(48);
    let s = t.f.call(Service::QueryInformationProcess, &[h, 0, buf, 48, 0]);
    (s, t.f.u32_at(buf), t.f.u64_at(buf + 32))
}

#[test]
fn create_user_process_starts_a_child_with_its_parameters() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/app.exe", &crate::tests::test_exe(false));
    let (params, want) = parameters_in(&mut t, "C:\\app.exe", "app.exe one \"two words\"", "C:\\Work");
    let (s, ph, th) = create_process(&mut t, params, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert!(ph != 0 && th != 0 && ph != th);
    // The child got its own address space with its parameters in it.
    let (child, mem) = t.f.children.last_mut().unwrap();
    let mut pp = [0u8; 8];
    mem.read(child.peb + crate::params::PEB_PROCESS_PARAMETERS, &mut pp).unwrap();
    let got =
        crate::params::read(u64::from_le_bytes(pp), &mut |a, b| mem.read(a, b).map_err(|_| STATUS_ACCESS_VIOLATION));
    assert_eq!(got, Ok(want));
    let pid = child.pid;
    // Running: the exit status is pending and the handle not signalled.
    let (s, exit, id) = exit_status_of(&mut t, ph);
    assert_eq!((s, exit, id), (STATUS_SUCCESS, STATUS_PENDING.0, u64::from(pid.0)));
    let zero = t.f.put_u64(0);
    assert_eq!(t.f.call(Service::WaitForSingleObject, &[ph, 0, zero]), STATUS_TIMEOUT);
    // Its last thread ends with 0x2A: the process's status, its handle
    // signalled, and the status kept while the handle is open.
    let (mut child, mut mem) = t.f.children.pop().unwrap();
    let tid = child.threads[0].thread;
    assert!(crate::exec::thread_exit(&mut t.f.sys, &mut child, &mut mem, tid, NtStatus(0x2A)));
    crate::exec::process_exit(&mut t.f.sys, &mut child, &mut mem);
    assert_eq!(t.f.call(Service::WaitForSingleObject, &[ph, 0, zero]), STATUS_SUCCESS);
    assert_eq!(t.f.call(Service::WaitForSingleObject, &[th, 0, zero]), STATUS_SUCCESS);
    assert_eq!(exit_status_of(&mut t, ph).1, 0x2A);
    t.close(th);
    t.close(ph);
    assert!(t.f.sys.process_exits.is_empty(), "forgotten with the process object");
}

#[test]
fn create_user_process_failures() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/app.exe", &crate::tests::test_exe(false));
    t.vfs_create(b"/nt/c/notpe.exe", b"MZ but nothing else");
    let (p, _) = parameters_in(&mut t, "C:\\missing.exe", "missing", "");
    assert_eq!(create_process(&mut t, p, 0).0, STATUS_OBJECT_NAME_NOT_FOUND);
    let (p, _) = parameters_in(&mut t, "app.exe", "app", "");
    assert_eq!(create_process(&mut t, p, 0).0, STATUS_OBJECT_PATH_SYNTAX_BAD);
    let (p, _) = parameters_in(&mut t, "C:\\notpe.exe", "notpe", "");
    assert_eq!(create_process(&mut t, p, 0).0, STATUS_INVALID_IMAGE_FORMAT);
    assert_eq!(create_process(&mut t, 0, 0).0, STATUS_INVALID_PARAMETER);
    // The NT path rules.
    let nt = |s: &str| {
        crate::threads::image_nt_name(&s.encode_utf16().collect::<Vec<_>>()).map(|v| String::from_utf16(&v).unwrap())
    };
    assert_eq!(nt("C:\\a\\b.exe"), Ok("\\??\\C:\\a\\b.exe".into()));
    assert_eq!(nt("c:/a/b.exe"), Ok("\\??\\c:\\a\\b.exe".into()));
    assert_eq!(nt("\\\\?\\C:\\x.exe"), Ok("\\??\\C:\\x.exe".into()));
    assert_eq!(nt("\\??\\Z:\\x.exe"), Ok("\\??\\Z:\\x.exe".into()));
    assert_eq!(nt("\\\\server\\share\\x.exe"), Err(STATUS_OBJECT_PATH_SYNTAX_BAD));
    assert!(t.f.children.is_empty());
}

/// `NtSetInformationObject` / `NtQueryObject` of Poly's
/// `ObjectHandleFlagInformation` (ADR 0089).
fn set_handle_flags(t: &mut T, h: u64, inherit: bool, protect: bool) -> NtStatus {
    let info = t.f.put(&[u8::from(inherit), u8::from(protect)]);
    t.f.call(Service::SetInformationObject, &[h, 4, info, 2])
}

fn handle_flags(t: &mut T, h: u64) -> (NtStatus, [u8; 2]) {
    let (buf, ret) = (t.f.alloc(8), t.f.alloc(4));
    let s = t.f.call(Service::QueryObject, &[h, 4, buf, 2, ret]);
    let mut b = [0u8; 2];
    t.f.mem.read(buf, &mut b).unwrap();
    (s, b)
}

#[test]
fn handle_flags_are_set_and_read() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\flags.txt");
    assert_eq!(handle_flags(&mut t, h), (STATUS_SUCCESS, [0, 0]));
    assert_eq!(set_handle_flags(&mut t, h, true, true), STATUS_SUCCESS);
    assert_eq!(handle_flags(&mut t, h), (STATUS_SUCCESS, [1, 1]));
    // Protected from close until the flag is cleared.
    assert_eq!(t.f.call(Service::Close, &[h]), STATUS_HANDLE_NOT_CLOSABLE);
    assert_eq!(set_handle_flags(&mut t, h, true, false), STATUS_SUCCESS);
    assert_eq!(handle_flags(&mut t, h).1, [1, 0]);
    // The wrong length, class or handle.
    let info = t.f.put(&[1, 0]);
    assert_eq!(t.f.call(Service::SetInformationObject, &[h, 4, info, 4]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.f.call(Service::SetInformationObject, &[h, 0, info, 2]), STATUS_INVALID_INFO_CLASS);
    assert_eq!(t.f.call(Service::SetInformationObject, &[0x7777_0000, 4, info, 2]), STATUS_INVALID_HANDLE);
    assert_eq!(t.f.call(Service::SetInformationObject, &[u64::MAX, 4, info, 2]), STATUS_INVALID_HANDLE);
    let (buf, ret) = (t.f.alloc(8), t.f.alloc(4));
    assert_eq!(t.f.call(Service::QueryObject, &[h, 4, buf, 1, ret]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.f.u32_at(ret), 2);
    t.close(h);
}

fn create_process_with(t: &mut T, params: u64, process_flags: u32, thread_flags: u32) -> (NtStatus, u64, u64) {
    let (ph, th) = (t.f.alloc(8), t.f.alloc(8));
    let args = [ph, th, 0, 0, 0, 0, u64::from(process_flags), u64::from(thread_flags), params, 0, 0];
    let s = t.f.call(Service::CreateUserProcess, &args);
    (s, t.f.u64_at(ph), t.f.u64_at(th))
}

#[test]
fn create_user_process_inherits_inheritable_handles_at_the_same_values() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/app.exe", &crate::tests::test_exe(false));
    let kept = t.file("\\??\\C:\\inherited.txt");
    let private = t.file("\\??\\C:\\private.txt");
    assert_eq!(set_handle_flags(&mut t, kept, true, false), STATUS_SUCCESS);
    let me = poly_ntobj::Caller::user(crate::tests::PID, crate::tests::TID);
    let object_of =
        |t: &T, c: &poly_ntobj::Caller, h: u64| t.f.sys.objects.object_by_handle(c, poly_ntobj::Handle(h), 0, None);
    let kept_object = object_of(&t, &me, kept).unwrap();

    // bInheritHandles: the inheritable handle at its value, still
    // inheritable; the other one not there.
    let (params, _) = parameters_in(&mut t, "C:\\app.exe", "app", "");
    let inherit = crate::consts::PROCESS_CREATE_FLAGS_INHERIT_HANDLES;
    let (s, ph, th) = create_process_with(&mut t, params, inherit, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (child, _) = t.f.children.last().unwrap();
    let them = poly_ntobj::Caller::user(child.pid, child.threads[0].thread);
    assert_eq!(object_of(&t, &them, kept), Ok(kept_object));
    assert!(t.f.sys.objects.handle_flags(&them, poly_ntobj::Handle(kept)).unwrap().inherit);
    assert_eq!(object_of(&t, &them, private), Err(STATUS_INVALID_HANDLE));
    // Its handle to the file counts: the creator may close its own.
    t.close(kept);
    assert_eq!(object_of(&t, &them, kept), Ok(kept_object));
    t.close(th);
    t.close(ph);

    // Without the flag, nothing is inherited.
    let again = t.file("\\??\\C:\\inherited.txt");
    assert_eq!(set_handle_flags(&mut t, again, true, false), STATUS_SUCCESS);
    let (s, ph, th) = create_process_with(&mut t, params, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (child, _) = t.f.children.last().unwrap();
    let them = poly_ntobj::Caller::user(child.pid, child.threads[0].thread);
    assert_eq!(object_of(&t, &them, again), Err(STATUS_INVALID_HANDLE));
    assert_eq!(t.f.sys.objects.process_handle_count(child.pid), Some(0));
    t.close(th);
    t.close(ph);
    t.close(again);
    t.close(private);
}

#[test]
fn create_user_process_suspended_and_with_a_startupinfo() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/app.exe", &crate::tests::test_exe(false));
    // A STARTUPINFO with STARTF_USESTDHANDLES and its three handles.
    let mut p = crate::params::Parameters {
        image_path: "C:\\app.exe".encode_utf16().collect(),
        command_line: "app".encode_utf16().collect(),
        environment: "A=1\0\0".encode_utf16().collect(),
        ..crate::params::Parameters::default()
    };
    p.startup.fixed[0..4].copy_from_slice(&104u32.to_le_bytes());
    p.startup.fixed[0x20..0x24].copy_from_slice(&crate::params::STARTF_USESTDHANDLES.to_le_bytes());
    p.startup.title = Some("child".encode_utf16().collect());
    p.std_handles = [0x10, 0x14, 0x14];
    let size = crate::params::image(0, &p).unwrap().len();
    let at = t.f.alloc(size);
    let bytes = crate::params::image(at, &p).unwrap();
    t.f.mem.write_bytes(at, &bytes).unwrap();

    let suspended = crate::consts::THREAD_CREATE_FLAGS_CREATE_SUSPENDED;
    let (s, ph, th) = create_process_with(&mut t, at, 0, suspended);
    assert_eq!(s, STATUS_SUCCESS);
    // The child's parameters hold the STARTUPINFO and the handles.
    let (child, mem) = t.f.children.last_mut().unwrap();
    let tid = child.threads[0].thread;
    let mut pp = [0u8; 8];
    mem.read(child.peb + crate::params::PEB_PROCESS_PARAMETERS, &mut pp).unwrap();
    let got =
        crate::params::read(u64::from_le_bytes(pp), &mut |a, b| mem.read(a, b).map_err(|_| STATUS_ACCESS_VIOLATION))
            .unwrap();
    assert_eq!(got, p);
    assert_eq!(got.given_std_handles(), Some([0x10, 0x14, 0x14]));
    // Its first thread waits for NtResumeThread on the creator's handle.
    assert_eq!(t.f.suspend.get(&tid), Some(&1));
    let prev = t.f.alloc(4);
    assert_eq!(t.f.call(Service::ResumeThread, &[th, prev]), STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(prev), 1);
    assert_eq!(t.f.suspend.get(&tid), Some(&0));
    t.close(th);
    t.close(ph);
}

fn create_section(t: &mut T, file: u64, size: u64, protect: u32) -> (NtStatus, u64) {
    let out = t.f.alloc(8);
    let max = if size == 0 { 0 } else { t.f.put_u64(size) };
    let s = t.f.call(
        Service::CreateSection,
        &[out, 0xF001F, 0, max, u64::from(protect), u64::from(crate::consts::SEC_COMMIT), file],
    );
    (s, t.f.u64_at(out))
}

fn map(t: &mut T, section: u64, protect: u32) -> (NtStatus, u64, u64) {
    let base = t.f.alloc(8);
    let size = t.f.alloc(8);
    let s = t
        .f
        .call(Service::MapViewOfSection, &[section, crate::tests::SELF, base, 0, 0, 0, size, 1, 0, u64::from(protect)]);
    (s, t.f.u64_at(base), t.f.u64_at(size))
}

#[test]
fn data_sections_of_files_map_their_contents_and_grow_them() {
    use crate::consts::{PAGE_READONLY, PAGE_READWRITE};
    let mut t = setup();
    t.vfs_create(b"/nt/c/data.bin", b"hello, mapping");
    let h = t.file("\\??\\C:\\data.bin");
    // The file's size by default.
    let (s, sec) = create_section(&mut t, h, 0, PAGE_READONLY);
    assert_eq!(s, STATUS_SUCCESS);
    let (s, base, size) = map(&mut t, sec, PAGE_READONLY);
    assert_eq!((s, size), (STATUS_SUCCESS, 0x1000));
    assert_eq!(t.f.bytes_at(base, 14), b"hello, mapping");
    // Flushing a view: rounded to pages; outside a view it fails.
    let (b, n, iosb) = (t.f.put_u64(base + 3), t.f.put_u64(10), t.f.alloc(16));
    assert_eq!(t.f.call(Service::FlushVirtualMemory, &[crate::tests::SELF, b, n, iosb]), STATUS_SUCCESS);
    assert_eq!((t.f.u64_at(b), t.f.u64_at(n)), (base, 0x1000));
    let (b, n) = (t.f.put_u64(0x1000), t.f.put_u64(0));
    assert_eq!(t.f.call(Service::FlushVirtualMemory, &[crate::tests::SELF, b, n, iosb]), STATUS_NOT_MAPPED_VIEW);
    // A read-only section cannot be larger than the file...
    assert_eq!(create_section(&mut t, h, 0x3000, PAGE_READONLY).0, STATUS_SECTION_TOO_BIG);
    // ... a writable one grows it (Learn: CreateFileMappingW).
    let (s, sec2) = create_section(&mut t, h, 0x3000, PAGE_READWRITE);
    assert_eq!(s, STATUS_SUCCESS);
    let (_, st) = t.query(h, FILE_STANDARD_INFORMATION, 24);
    assert_eq!(u64::from_le_bytes(st[8..16].try_into().unwrap()), 0x3000);
    // A file opened only for reading cannot back a writable section.
    let (s, ro, _) = t.create("\\??\\C:\\data.bin", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(create_section(&mut t, ro, 0, PAGE_READWRITE).0, STATUS_ACCESS_DENIED);
    let (s, sec3) = create_section(&mut t, ro, 0, PAGE_READONLY);
    assert_eq!(s, STATUS_SUCCESS);
    // An empty file needs a size.
    t.vfs_create(b"/nt/c/empty.bin", b"");
    let e = t.file("\\??\\C:\\empty.bin");
    assert_eq!(create_section(&mut t, e, 0, PAGE_READWRITE).0, STATUS_MAPPED_FILE_SIZE_ZERO);
    for h in [sec, sec2, sec3, ro, e, h] {
        t.close(h);
    }
}
