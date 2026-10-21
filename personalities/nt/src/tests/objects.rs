use super::*;
use crate::consts::*;
use poly_ntobj::access::*;
use poly_ntobj::status::*;
use poly_ntobj::{OBJ_CASE_INSENSITIVE, OBJ_OPENIF};
use std::string::String;

const CI: u32 = OBJ_CASE_INSENSITIVE;
const EVENT: &str = "\\BaseNamedObjects\\PolyTestEvent";

fn create_event(f: &mut Fake, name: &str, attrs: u32) -> (NtStatus, u64) {
    let out = f.alloc(8);
    let oa = if name.is_empty() { 0 } else { f.attrs(name, attrs) };
    let s = f.call(Service::CreateEvent, &[out, u64::from(EVENT_ALL_ACCESS), oa, u64::from(NOTIFICATION_EVENT), 0]);
    (s, f.u64_at(out))
}

fn open_event(f: &mut Fake, name: &str) -> (NtStatus, u64) {
    let out = f.alloc(8);
    let oa = f.attrs(name, CI);
    let s = f.call(Service::OpenEvent, &[out, u64::from(EVENT_ALL_ACCESS), oa]);
    (s, f.u64_at(out))
}

/// `(attributes, granted, handles, pointers)` from `ObjectBasicInformation`.
fn basic(f: &mut Fake, h: u64) -> [u32; 4] {
    let buf = f.alloc(OBJECT_BASIC_INFORMATION_SIZE);
    let ret = f.alloc(4);
    let s = f.call(Service::QueryObject, &[h, u64::from(OBJECT_BASIC_INFORMATION), buf, 56, ret]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(f.u32_at(ret), 56);
    [f.u32_at(buf), f.u32_at(buf + 4), f.u32_at(buf + 8), f.u32_at(buf + 12)]
}

fn unicode_at(f: &Fake, addr: u64) -> String {
    let len = u16::from_le_bytes(f.bytes_at(addr, 2).try_into().unwrap()) as usize;
    let buf = f.u64_at(addr + 8);
    let bytes = f.bytes_at(buf, len);
    String::from_utf16(&bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap()
}

fn query_string(f: &mut Fake, h: u64, class: u32) -> String {
    let buf = f.alloc(512);
    let ret = f.alloc(4);
    assert_eq!(f.call(Service::QueryObject, &[h, u64::from(class), buf, 512, ret]), STATUS_SUCCESS);
    unicode_at(f, buf)
}

#[test]
fn named_event_lifetime_through_handles() {
    let mut f = Fake::new();
    let (s, h1) = create_event(&mut f, EVENT, CI);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(h1 % 4, 0);
    let (s, h2) = open_event(&mut f, "\\basenamedobjects\\polytestevent");
    assert_eq!(s, STATUS_SUCCESS);
    assert_ne!(h1, h2);
    assert_eq!(basic(&mut f, h1)[2], 2);
    // Create again: a collision, or the same object with OBJ_OPENIF.
    assert_eq!(create_event(&mut f, EVENT, CI).0, STATUS_OBJECT_NAME_COLLISION);
    let (s, h3) = create_event(&mut f, EVENT, CI | OBJ_OPENIF);
    assert_eq!(s, STATUS_OBJECT_NAME_EXISTS);
    assert_eq!(basic(&mut f, h1)[2], 3);
    // Duplicate one, then close everything: the event is gone.
    let out = f.alloc(8);
    let dup =
        f.call(Service::DuplicateObject, &[SELF, h2, SELF, out, 0, 0, u64::from(poly_ntobj::DUPLICATE_SAME_ACCESS)]);
    assert_eq!(dup, STATUS_SUCCESS);
    let h4 = f.u64_at(out);
    let info = basic(&mut f, h4);
    assert_eq!((info[1], info[2]), (EVENT_ALL_ACCESS, 4));
    assert_eq!(query_string(&mut f, h4, OBJECT_NAME_INFORMATION), EVENT);
    assert_eq!(query_string(&mut f, h4, OBJECT_TYPE_INFORMATION), "Event");
    let objects = f.sys.objects.object_count();
    for h in [h1, h2, h3] {
        assert_eq!(f.call(Service::Close, &[h]), STATUS_SUCCESS);
    }
    assert_eq!(f.sys.objects.object_count(), objects);
    assert_eq!(f.call(Service::Close, &[h4]), STATUS_SUCCESS);
    assert_eq!(f.sys.objects.object_count(), objects - 1);
    assert_eq!(open_event(&mut f, EVENT).0, STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(f.call(Service::Close, &[h4]), STATUS_INVALID_HANDLE);
}

#[test]
fn set_and_reset_report_the_previous_state() {
    let mut f = Fake::new();
    let (_, h) = create_event(&mut f, "", 0);
    let prev = f.alloc(4);
    assert_eq!(f.call(Service::SetEvent, &[h, prev]), STATUS_SUCCESS);
    assert_eq!(f.u32_at(prev), 0);
    assert_eq!(f.call(Service::SetEvent, &[h, 0]), STATUS_SUCCESS);
    assert_eq!(f.call(Service::ResetEvent, &[h, prev]), STATUS_SUCCESS);
    assert_eq!(f.u32_at(prev), 1);
    assert_eq!(query_string(&mut f, h, OBJECT_NAME_INFORMATION), "");
    let out = f.alloc(8);
    let bad = f.call(Service::CreateEvent, &[out, u64::from(EVENT_ALL_ACCESS), 0, 9, 0]);
    assert_eq!(bad, STATUS_INVALID_PARAMETER);
}

#[test]
fn pseudo_handles_duplicate_to_process_and_thread() {
    let mut f = Fake::new();
    let out = f.alloc(8);
    let same = u64::from(poly_ntobj::DUPLICATE_SAME_ACCESS);
    assert_eq!(f.call(Service::DuplicateObject, &[SELF, SELF, SELF, out, 0, 0, same]), STATUS_SUCCESS);
    let process = f.u64_at(out);
    assert_eq!(query_string(&mut f, process, OBJECT_TYPE_INFORMATION), "Process");
    assert_eq!(f.call(Service::DuplicateObject, &[SELF, SELF - 1, SELF, out, 0, 0, same]), STATUS_SUCCESS);
    let thread = f.u64_at(out);
    assert_eq!(query_string(&mut f, thread, OBJECT_TYPE_INFORMATION), "Thread");
    // The real process handle works where NtCurrentProcess() does.
    let base = f.put_u64(0);
    let size = f.put_u64(0x1000);
    let s = f.call(
        Service::AllocateVirtualMemory,
        &[process, base, 0, size, u64::from(MEM_COMMIT), u64::from(PAGE_READWRITE)],
    );
    assert_eq!(s, STATUS_SUCCESS);
    // Closing a pseudo handle does nothing.
    assert_eq!(f.call(Service::Close, &[SELF]), STATUS_SUCCESS);
}

#[test]
fn query_object_errors() {
    let mut f = Fake::new();
    let (_, h) = create_event(&mut f, EVENT, 0);
    let buf = f.alloc(64);
    let ret = f.alloc(4);
    assert_eq!(f.call(Service::QueryObject, &[h, 0, buf, 20, ret]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(f.u32_at(ret), 56);
    assert_eq!(f.call(Service::QueryObject, &[h, 1, buf, 16, ret]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(f.u32_at(ret) as usize, 16 + EVENT.len() * 2 + 2);
    assert_eq!(f.call(Service::QueryObject, &[h, 9, buf, 64, ret]), STATUS_INVALID_INFO_CLASS);
    assert_eq!(f.call(Service::QueryObject, &[0x4444, 0, buf, 64, ret]), STATUS_INVALID_HANDLE);
    // A bad output pointer is an access violation.
    assert_eq!(f.call(Service::QueryObject, &[h, 0, 0x5000, 64, 0]), STATUS_ACCESS_VIOLATION);
}

fn open_directory(f: &mut Fake, name: &str) -> (NtStatus, u64) {
    let out = f.alloc(8);
    let oa = f.attrs(name, CI);
    let s = f.call(Service::OpenDirectoryObject, &[out, u64::from(DIRECTORY_QUERY | DIRECTORY_TRAVERSE), oa]);
    (s, f.u64_at(out))
}

/// Every `(name, type)` of a directory, one call at a time.
fn list_directory(f: &mut Fake, h: u64, single: bool) -> Vec<(String, String)> {
    let buf = f.alloc(1024);
    let ctx = f.alloc(4);
    let ret = f.alloc(4);
    let mut out = Vec::new();
    let mut restart = 1;
    loop {
        let s = f.call(Service::QueryDirectoryObject, &[h, buf, 1024, u64::from(single), restart, ctx, ret]);
        restart = 0;
        if s == STATUS_NO_MORE_ENTRIES {
            return out;
        }
        assert!(s == STATUS_SUCCESS || s == STATUS_MORE_ENTRIES, "{s:?}");
        let mut at = buf;
        while f.u64_at(at + 8) != 0 {
            out.push((unicode_at(f, at), unicode_at(f, at + 16)));
            at += 32;
        }
    }
}

#[test]
fn directories_and_symbolic_links() {
    let mut f = Fake::new();
    let (s, root) = open_directory(&mut f, "\\");
    assert_eq!(s, STATUS_SUCCESS);
    let all = list_directory(&mut f, root, false);
    for name in ["??", "BaseNamedObjects", "Device", "KnownDlls"] {
        assert!(all.iter().any(|(n, t)| n == name && t == "Directory"), "{name} in {all:?}");
    }
    assert!(all.iter().any(|(n, t)| n == "DosDevices" && t == "SymbolicLink"));
    assert_eq!(list_directory(&mut f, root, true), all);
    for d in ["\\??", "\\Device", "\\KnownDlls", "\\BaseNamedObjects"] {
        assert_eq!(open_directory(&mut f, d).0, STATUS_SUCCESS, "{d}");
    }

    // A directory of our own, an event in it, a link to it.
    let out = f.alloc(8);
    let oa = f.attrs("\\BaseNamedObjects\\PolyDir", CI);
    assert_eq!(f.call(Service::CreateDirectoryObject, &[out, u64::from(DIRECTORY_ALL_ACCESS), oa]), STATUS_SUCCESS);
    let dir = f.u64_at(out);
    let (s, _ev) = create_event(&mut f, "\\BaseNamedObjects\\PolyDir\\Inner", CI);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(list_directory(&mut f, dir, false), [(String::from("Inner"), String::from("Event"))]);
    let target = f.unicode("\\BaseNamedObjects\\PolyDir");
    let oa = f.attrs("\\BaseNamedObjects\\PolyLink", CI);
    let s = f.call(Service::CreateSymbolicLinkObject, &[out, u64::from(SYMBOLIC_LINK_ALL_ACCESS), oa, target]);
    assert_eq!(s, STATUS_SUCCESS);
    let link = f.u64_at(out);
    // Through the link.
    assert_eq!(open_event(&mut f, "\\BaseNamedObjects\\PolyLink\\Inner").0, STATUS_SUCCESS);
    // The link itself and its target.
    let oa = f.attrs("\\BaseNamedObjects\\PolyLink", CI);
    assert_eq!(f.call(Service::OpenSymbolicLinkObject, &[out, u64::from(SYMBOLIC_LINK_QUERY), oa]), STATUS_SUCCESS);
    let opened = f.u64_at(out);
    assert_eq!(query_string(&mut f, opened, OBJECT_TYPE_INFORMATION), "SymbolicLink");
    let buf = f.alloc(128);
    let us = f.put(&crate::abi::UnicodeString { length: 0, maximum_length: 128, buffer: buf }.to_bytes());
    let ret = f.alloc(4);
    assert_eq!(f.call(Service::QuerySymbolicLinkObject, &[link, us, ret]), STATUS_SUCCESS);
    assert_eq!(unicode_at(&f, us), "\\BaseNamedObjects\\PolyDir");
    assert_eq!(f.u32_at(ret) as usize, "\\BaseNamedObjects\\PolyDir".len() * 2 + 2);
    let small = f.put(&crate::abi::UnicodeString { length: 0, maximum_length: 4, buffer: buf }.to_bytes());
    assert_eq!(f.call(Service::QuerySymbolicLinkObject, &[link, small, ret]), STATUS_BUFFER_TOO_SMALL);
    assert_eq!(f.u32_at(ret) as usize, "\\BaseNamedObjects\\PolyDir".len() * 2);
    // A relative name under a directory handle.
    let oa = f.attrs_in(dir, "Inner", CI);
    assert_eq!(f.call(Service::OpenEvent, &[out, u64::from(EVENT_ALL_ACCESS), oa]), STATUS_SUCCESS);
    // \DosDevices leads to \??.
    assert_eq!(open_directory(&mut f, "\\DosDevices").0, STATUS_SUCCESS);
    assert_eq!(open_directory(&mut f, "\\Nowhere").0, STATUS_OBJECT_NAME_NOT_FOUND);
}

#[test]
fn query_directory_buffer_rules() {
    let mut f = Fake::new();
    let (_, root) = open_directory(&mut f, "\\");
    let buf = f.alloc(1024);
    let ctx = f.alloc(4);
    let ret = f.alloc(4);
    assert_eq!(f.call(Service::QueryDirectoryObject, &[root, buf, 8, 0, 1, ctx, ret]), STATUS_BUFFER_TOO_SMALL);
    assert!(f.u32_at(ret) > 64);
    // Just enough for one entry: more entries follow.
    let one = u64::from(f.u32_at(ret));
    assert_eq!(f.call(Service::QueryDirectoryObject, &[root, buf, one, 0, 1, ctx, ret]), STATUS_MORE_ENTRIES);
    assert_eq!(f.u32_at(ctx), 1);
    let (_, ev) = create_event(&mut f, "", 0);
    assert_eq!(f.call(Service::QueryDirectoryObject, &[ev, buf, 1024, 0, 1, ctx, ret]), STATUS_OBJECT_TYPE_MISMATCH);
}

#[test]
fn bad_object_attributes() {
    let mut f = Fake::new();
    let out = f.alloc(8);
    let oa = f.attrs(EVENT, 0);
    f.mem.write_bytes(oa, &40u32.to_le_bytes()).unwrap();
    assert_eq!(f.call(Service::OpenEvent, &[out, 0, oa]), STATUS_INVALID_PARAMETER);
    assert_eq!(f.call(Service::OpenEvent, &[out, 0, 0x7000]), STATUS_ACCESS_VIOLATION);
    // A handle that cannot be returned is closed again.
    let handles = f.sys.objects.process_handle_count(PID);
    let oa = f.attrs(EVENT, 0);
    let s = f.call(Service::CreateEvent, &[0x7000, u64::from(EVENT_ALL_ACCESS), oa, 0, 0]);
    assert_eq!(s, STATUS_ACCESS_VIOLATION);
    assert_eq!(f.sys.objects.process_handle_count(PID), handles);
    assert_eq!(open_event(&mut f, EVENT).0, STATUS_OBJECT_NAME_NOT_FOUND);
}
