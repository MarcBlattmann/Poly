//! The registry services through `dispatch_with`, against the fake kernel
//! (ADR 0066).

use super::*;
use crate::consts::*;
use poly_ntobj::access::{ACCESS_SYSTEM_SECURITY, EVENT_ALL_ACCESS, READ_CONTROL, WRITE_DAC};
use poly_ntobj::status::*;
use poly_ntobj::{Handle, OBJ_CASE_INSENSITIVE, OBJ_OPENLINK};
use poly_registry::consts::*;
use std::string::String;

const CI: u32 = OBJ_CASE_INSENSITIVE;
const SOFTWARE: &str = "\\Registry\\Machine\\SOFTWARE";

fn utf16(b: &[u8]) -> String {
    String::from_utf16(&b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap()
}

fn sz(s: &str) -> Vec<u8> {
    poly_registry::defaults::sz(s)
}

/// `NtCreateKey`: (status, handle, disposition).
fn create_key(f: &mut Fake, root: u64, name: &str, access: u32, options: u32) -> (NtStatus, u64, u32) {
    let out = f.alloc(8);
    let disp = f.alloc(4);
    let oa = f.attrs_in(root, name, CI);
    let s = f.call(Service::CreateKey, &[out, u64::from(access), oa, 0, 0, u64::from(options), disp]);
    (s, f.u64_at(out), f.u32_at(disp))
}

fn open_key(f: &mut Fake, root: u64, name: &str, access: u32) -> (NtStatus, u64) {
    let out = f.alloc(8);
    let oa = f.attrs_in(root, name, CI);
    let s = f.call(Service::OpenKey, &[out, u64::from(access), oa]);
    (s, f.u64_at(out))
}

fn open_ok(f: &mut Fake, name: &str) -> u64 {
    let (s, h) = open_key(f, 0, name, KEY_ALL_ACCESS);
    assert_eq!(s, STATUS_SUCCESS, "{name}");
    h
}

fn set_value(f: &mut Fake, h: u64, name: &str, kind: u32, data: &[u8]) -> NtStatus {
    let n = f.unicode(name);
    let d = if data.is_empty() { 0 } else { f.put(data) };
    f.call(Service::SetValueKey, &[h, n, 0, u64::from(kind), d, data.len() as u64])
}

/// `NtQueryValueKey`: (status, result length, bytes written region).
fn query_value(f: &mut Fake, h: u64, name: &str, class: u32, length: usize) -> (NtStatus, u32, Vec<u8>) {
    let n = f.unicode(name);
    let buf = f.alloc(length.max(8));
    let ret = f.alloc(4);
    let s = f.call(Service::QueryValueKey, &[h, n, u64::from(class), buf, length as u64, ret]);
    (s, f.u32_at(ret), f.bytes_at(buf, length))
}

fn close(f: &mut Fake, h: u64) {
    assert_eq!(f.call(Service::Close, &[h]), STATUS_SUCCESS);
}

fn key_name(f: &mut Fake, h: u64) -> String {
    let buf = f.alloc(512);
    let ret = f.alloc(4);
    assert_eq!(f.call(Service::QueryKey, &[h, u64::from(KEY_NAME_INFORMATION), buf, 512, ret]), STATUS_SUCCESS);
    let len = f.u32_at(buf) as usize;
    utf16(&f.bytes_at(buf + 4, len))
}

#[test]
fn registry_is_in_the_namespace() {
    let mut f = Fake::new();
    let h = open_ok(&mut f, "\\REGISTRY");
    assert_eq!(key_name(&mut f, h), "\\REGISTRY");
    let m = open_ok(&mut f, "\\registry\\machine");
    assert_eq!(key_name(&mut f, m), "\\REGISTRY\\MACHINE");
    // The object type is Key.
    let buf = f.alloc(256);
    let ret = f.alloc(4);
    assert_eq!(f.call(Service::QueryObject, &[m, u64::from(OBJECT_TYPE_INFORMATION), buf, 256, ret]), STATUS_SUCCESS);
    let len = u16::from_le_bytes(f.bytes_at(buf, 2).try_into().unwrap()) as usize;
    let at = f.u64_at(buf + 8);
    assert_eq!(utf16(&f.bytes_at(at, len)), "Key");
    close(&mut f, h);
    close(&mut f, m);
    // Not a key: another type through the registry (the object services
    // have no parse procedure: ADR 0030's NoParse), or a key outside it.
    let out = f.alloc(8);
    let oa = f.attrs("\\REGISTRY\\MACHINE", CI);
    assert_eq!(f.call(Service::OpenEvent, &[out, u64::from(EVENT_ALL_ACCESS), oa]), STATUS_OBJECT_PATH_NOT_FOUND);
    assert_eq!(open_key(&mut f, 0, "\\BaseNamedObjects", KEY_READ).0, STATUS_OBJECT_TYPE_MISMATCH);
    let objects = f.sys.objects.object_count();
    assert_eq!(create_key(&mut f, 0, "\\BaseNamedObjects\\NotAKey", KEY_ALL_ACCESS, 0).0, STATUS_OBJECT_TYPE_MISMATCH);
    assert_eq!(f.sys.objects.object_count(), objects, "nothing left behind");
    assert_eq!(f.sys.registry.reg.open_count(), 1, "only \\REGISTRY's own instance");
}

#[test]
fn create_set_query_enumerate() {
    let mut f = Fake::new();
    let (s, h, d) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\PolyTest", KEY_ALL_ACCESS, 0);
    assert_eq!((s, d), (STATUS_SUCCESS, REG_CREATED_NEW_KEY));
    let (s, h2, d) = create_key(&mut f, 0, "\\REGISTRY\\MACHINE\\software\\polytest", KEY_READ, 0);
    assert_eq!((s, d), (STATUS_SUCCESS, REG_OPENED_EXISTING_KEY));
    assert_ne!(h, h2);

    assert_eq!(set_value(&mut f, h, "Text", REG_SZ, &sz("hello")), STATUS_SUCCESS);
    assert_eq!(set_value(&mut f, h, "Number", REG_DWORD, &42u32.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(set_value(&mut f, h, "Big", REG_QWORD, &7u64.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(set_value(&mut f, h, "Blob", REG_BINARY, &[1, 2, 3]), STATUS_SUCCESS);
    assert_eq!(set_value(&mut f, h, "List", REG_MULTI_SZ, &[b'a', 0, 0, 0, 0, 0]), STATUS_SUCCESS);
    // The default value: a null name.
    let d = f.put(&sz("default"));
    assert_eq!(f.call(Service::SetValueKey, &[h, 0, 0, u64::from(REG_SZ), d, 16]), STATUS_SUCCESS);
    // A read-only handle may not set values.
    assert_eq!(set_value(&mut f, h2, "No", REG_SZ, &sz("x")), STATUS_ACCESS_DENIED);

    // KeyValuePartialInformation: TitleIndex, Type, DataLength, Data.
    let (s, len, b) = query_value(&mut f, h2, "number", KEY_VALUE_PARTIAL_INFORMATION, 64);
    assert_eq!((s, len), (STATUS_SUCCESS, 16));
    assert_eq!(&b[4..16], &[4, 0, 0, 0, 4, 0, 0, 0, 42, 0, 0, 0]);
    // Buffer sizes: header only, then too small.
    let (s, len, b) = query_value(&mut f, h2, "Text", KEY_VALUE_PARTIAL_INFORMATION, 12);
    assert_eq!((s, len), (STATUS_BUFFER_OVERFLOW, 12 + 12));
    assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 12, "DataLength is there");
    let (s, len, _) = query_value(&mut f, h2, "Text", KEY_VALUE_PARTIAL_INFORMATION, 11);
    assert_eq!((s, len), (STATUS_BUFFER_TOO_SMALL, 24));
    let (s, _, _) = query_value(&mut f, h2, "missing", KEY_VALUE_PARTIAL_INFORMATION, 64);
    assert_eq!(s, STATUS_OBJECT_NAME_NOT_FOUND);
    let (s, _, _) = query_value(&mut f, h2, "Text", 77, 64);
    assert_eq!(s, STATUS_INVALID_PARAMETER);
    // The default value by its empty name.
    let (s, _, b) = query_value(&mut f, h2, "", KEY_VALUE_PARTIAL_INFORMATION, 64);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(&b[12..28], &sz("default")[..]);
    // Full information: name then data at DataOffset, the next multiple of 8
    // (NtQueryValueKey.yaml `full`).
    let (s, len, b) = query_value(&mut f, h2, "BLOB", KEY_VALUE_FULL_INFORMATION, 64);
    assert_eq!((s, len), (STATUS_SUCCESS, 32 + 3));
    assert_eq!(utf16(&b[20..28]), "Blob");
    assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 32);
    assert_eq!(&b[32..35], &[1, 2, 3]);

    // Enumerate the values, in the order they were set.
    let mut names = Vec::new();
    for i in 0.. {
        let buf = f.alloc(128);
        let ret = f.alloc(4);
        let s = f.call(Service::EnumerateValueKey, &[h2, i, u64::from(KEY_VALUE_BASIC_INFORMATION), buf, 128, ret]);
        if s == STATUS_NO_MORE_ENTRIES {
            break;
        }
        assert_eq!(s, STATUS_SUCCESS);
        let n = f.u32_at(buf + 8) as usize;
        names.push(utf16(&f.bytes_at(buf + 12, n)));
    }
    assert_eq!(names, ["Text", "Number", "Big", "Blob", "List", ""]);

    // KeyFullInformation counts.
    let buf = f.alloc(128);
    let ret = f.alloc(4);
    assert_eq!(f.call(Service::QueryKey, &[h2, u64::from(KEY_FULL_INFORMATION), buf, 128, ret]), STATUS_SUCCESS);
    assert_eq!(f.u32_at(buf + 32), 6, "Values");
    assert_eq!(f.u32_at(buf + 20), 0, "SubKeys");
    // Too small for even the fixed part.
    assert_eq!(
        f.call(Service::QueryKey, &[h2, u64::from(KEY_FULL_INFORMATION), buf, 10, ret]),
        STATUS_BUFFER_TOO_SMALL
    );
    assert_eq!(f.u32_at(ret), 44);

    // The new key is among SOFTWARE's subkeys.
    let sw = open_ok(&mut f, SOFTWARE);
    let mut subs = Vec::new();
    for i in 0.. {
        let buf = f.alloc(128);
        let ret = f.alloc(4);
        let s = f.call(Service::EnumerateKey, &[sw, i, u64::from(KEY_BASIC_INFORMATION), buf, 128, ret]);
        if s == STATUS_NO_MORE_ENTRIES {
            break;
        }
        assert_eq!(s, STATUS_SUCCESS);
        let n = f.u32_at(buf + 12) as usize;
        subs.push(utf16(&f.bytes_at(buf + 16, n)));
    }
    // (`Wow6432Node` is the 32-bit view of the software hive, ADR 0186 stage 2: a 64-bit caller
    // lists it; the other names are a Windows machine's defaults, `RegistryDefaults.Wow6432Node.yaml`.)
    assert_eq!(
        subs,
        ["Classes", "Clients", "Microsoft", "Policies", "PolyTest", "RegisteredApplications", "WOW6432Node"]
    );
    let buf = f.alloc(64);
    assert_eq!(
        f.call(Service::EnumerateKey, &[sw, 0, u64::from(KEY_NAME_INFORMATION), buf, 64, 0]),
        STATUS_INVALID_PARAMETER
    );

    // Delete a value, then the key.
    let n = f.unicode("Blob");
    assert_eq!(f.call(Service::DeleteValueKey, &[h, n]), STATUS_SUCCESS);
    assert_eq!(f.call(Service::DeleteValueKey, &[h, n]), STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(f.call(Service::DeleteKey, &[h2]), STATUS_ACCESS_DENIED, "KEY_READ has no DELETE");
    assert_eq!(f.call(Service::DeleteKey, &[h]), STATUS_SUCCESS);
    assert_eq!(query_value(&mut f, h2, "Text", KEY_VALUE_PARTIAL_INFORMATION, 64).0, STATUS_KEY_DELETED);
    assert_eq!(
        open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\PolyTest", KEY_READ).0,
        STATUS_OBJECT_NAME_NOT_FOUND
    );
    for x in [h, h2, sw] {
        close(&mut f, x);
    }
    assert_eq!(f.sys.registry.reg.open_count(), 1, "every instance ended");
}

#[test]
fn relative_names_and_create_access() {
    let mut f = Fake::new();
    let sw = open_ok(&mut f, SOFTWARE);
    let (s, a, _) = create_key(&mut f, sw, "Poly", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (s, b, _) = create_key(&mut f, a, "Inner", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, b), "\\REGISTRY\\MACHINE\\SOFTWARE\\Poly\\Inner");
    // Only the last component is created.
    assert_eq!(create_key(&mut f, sw, "No\\Such", KEY_ALL_ACCESS, 0).0, STATUS_OBJECT_NAME_NOT_FOUND);
    // A root handle without KEY_CREATE_SUB_KEY may open but not create.
    let (s, ro) = open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Poly", KEY_READ);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(create_key(&mut f, ro, "New", KEY_ALL_ACCESS, 0).0, STATUS_ACCESS_DENIED);
    let (s, _, d) = create_key(&mut f, ro, "Inner", KEY_READ, 0);
    assert_eq!((s, d), (STATUS_SUCCESS, REG_OPENED_EXISTING_KEY));
    assert_eq!(open_key(&mut f, ro, "inner", KEY_READ).0, STATUS_SUCCESS);
    // An empty name with a root opens the root key again.
    assert_eq!(open_key(&mut f, ro, "", KEY_READ).0, STATUS_SUCCESS);
    // Bad options.
    assert_eq!(create_key(&mut f, sw, "X", KEY_ALL_ACCESS, 0x100).0, STATUS_INVALID_PARAMETER);
    let out = f.alloc(8);
    let oa = f.attrs(SOFTWARE, CI);
    assert_eq!(f.call(Service::OpenKeyEx, &[out, u64::from(KEY_READ), oa, 1]), STATUS_INVALID_PARAMETER_4);
    // KEY_WOW64_64KEY is not a right: dropped.
    assert_eq!(open_key(&mut f, 0, SOFTWARE, KEY_READ | KEY_WOW64_64KEY).0, STATUS_SUCCESS);
    // Volatile rules: no non-volatile key below a volatile one, none below
    // a master key.
    let (s, v, _) = create_key(&mut f, a, "Vol", KEY_ALL_ACCESS, REG_OPTION_VOLATILE);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(create_key(&mut f, v, "NonVol", KEY_ALL_ACCESS, 0).0, STATUS_CHILD_MUST_BE_VOLATILE);
    assert_eq!(create_key(&mut f, 0, "\\Registry\\Machine\\Mine", KEY_ALL_ACCESS, 0).0, STATUS_CHILD_MUST_BE_VOLATILE);
}

#[test]
fn current_control_set_link_and_own_links() {
    let mut f = Fake::new();
    let h = open_ok(&mut f, "\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Control\\ComputerName\\ComputerName");
    assert_eq!(key_name(&mut f, h), "\\REGISTRY\\MACHINE\\SYSTEM\\ControlSet001\\Control\\ComputerName\\ComputerName");
    let (s, _, b) = query_value(&mut f, h, "ComputerName", KEY_VALUE_PARTIAL_INFORMATION, 64);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(&b[12..22], &sz("POLY")[..]);
    // The link itself.
    let out = f.alloc(8);
    let oa = f.attrs("\\Registry\\Machine\\SYSTEM\\CurrentControlSet", CI);
    assert_eq!(
        f.call(Service::OpenKeyEx, &[out, u64::from(KEY_READ), oa, u64::from(REG_OPTION_OPEN_LINK)]),
        STATUS_SUCCESS
    );
    let link = f.u64_at(out);
    assert_eq!(key_name(&mut f, link), "\\REGISTRY\\MACHINE\\SYSTEM\\CurrentControlSet");
    let oa = f.attrs("\\Registry\\Machine\\SYSTEM\\CurrentControlSet", CI | OBJ_OPENLINK);
    assert_eq!(f.call(Service::OpenKey, &[out, u64::from(KEY_READ), oa]), STATUS_SUCCESS);

    // Our own link, made as the documentation says.
    let (s, l, _) = create_key(
        &mut f,
        0,
        "\\Registry\\Machine\\SOFTWARE\\PolyLink",
        KEY_ALL_ACCESS | KEY_CREATE_LINK,
        REG_OPTION_CREATE_LINK | REG_OPTION_VOLATILE,
    );
    assert_eq!(s, STATUS_SUCCESS);
    let target: Vec<u8> =
        "\\Registry\\Machine\\SOFTWARE\\Microsoft".encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(set_value(&mut f, l, "SymbolicLinkValue", REG_LINK, &target), STATUS_SUCCESS);
    let through = open_ok(&mut f, "\\Registry\\Machine\\SOFTWARE\\PolyLink\\Windows NT\\CurrentVersion");
    assert_eq!(key_name(&mut f, through), "\\REGISTRY\\MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
    let (s, _, b) = query_value(&mut f, through, "ProductName", KEY_VALUE_PARTIAL_INFORMATION, 64);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(&b[12..22], &sz("Poly")[..]);
}

/// Create an unnamed notification event: its handle.
fn event(f: &mut Fake) -> u64 {
    let out = f.alloc(8);
    let s = f.call(Service::CreateEvent, &[out, u64::from(EVENT_ALL_ACCESS), 0, u64::from(NOTIFICATION_EVENT), 1]);
    assert_eq!(s, STATUS_SUCCESS);
    f.u64_at(out)
}

fn signaled(f: &Fake, h: u64) -> bool {
    f.sys.objects.nt_query_event(&f.caller(), Handle(h)).unwrap().signaled
}

fn notify(f: &mut Fake, key: u64, ev: u64, iosb: u64, filter: u32, tree: bool) -> NtStatus {
    f.call(Service::NotifyChangeKey, &[key, ev, 0, 0, iosb, u64::from(filter), u64::from(tree), 0, 0, 1])
}

#[test]
fn notifications_signal_events_and_write_status_blocks() {
    let mut f = Fake::new();
    let (_, k, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Watched", KEY_ALL_ACCESS, 0);
    let ev = event(&mut f);
    let iosb = f.alloc(16);
    f.mem.write_bytes(iosb, &[0xAA; 16]).unwrap();
    assert_eq!(notify(&mut f, k, ev, iosb, REG_NOTIFY_CHANGE_LAST_SET, false), STATUS_PENDING);
    assert!(!signaled(&f, ev), "the event is reset when the request starts");
    assert_eq!(f.u32_at(iosb), 0xAAAA_AAAA, "not written yet");
    // A subkey does not match the filter.
    let (_, sub, _) = create_key(&mut f, k, "Sub", KEY_ALL_ACCESS, 0);
    assert!(!signaled(&f, ev));
    assert_eq!(set_value(&mut f, k, "v", REG_DWORD, &[1, 0, 0, 0]), STATUS_SUCCESS);
    assert!(signaled(&f, ev));
    assert_eq!(f.u32_at(iosb), STATUS_SUCCESS.0);
    assert_eq!(f.u64_at(iosb + 8), 0);

    // A subtree watch sees the subkey's change.
    let iosb2 = f.alloc(16);
    assert_eq!(notify(&mut f, k, ev, iosb2, REG_NOTIFY_CHANGE_LAST_SET, true), STATUS_PENDING);
    assert!(!signaled(&f, ev));
    // The filter was set by the first call; the subtree flag too (Learn).
    assert_eq!(set_value(&mut f, sub, "w", REG_DWORD, &[1, 0, 0, 0]), STATUS_SUCCESS);
    assert!(!signaled(&f, ev), "the first call's WatchTree = FALSE still holds");
    // Closing the handle completes the request with STATUS_NOTIFY_CLEANUP.
    close(&mut f, k);
    assert!(signaled(&f, ev));
    assert_eq!(f.u32_at(iosb2), STATUS_NOTIFY_CLEANUP.0);
    assert_eq!(f.sys.registry.pending_requests(), 0);

    // A fresh handle with a subtree watch.
    let k = open_ok(&mut f, "\\Registry\\Machine\\SOFTWARE\\Watched");
    let iosb3 = f.alloc(16);
    assert_eq!(notify(&mut f, k, ev, iosb3, REG_NOTIFY_CHANGE_NAME, true), STATUS_PENDING);
    assert_eq!(f.call(Service::DeleteKey, &[sub]), STATUS_SUCCESS);
    assert!(signaled(&f, ev));
    assert_eq!(f.u32_at(iosb3), STATUS_SUCCESS.0);

    // Not supported yet, or wrong.
    let iosb4 = f.alloc(16);
    // Synchronous: the fake kernel's waits time out at once, so the request
    // is taken back and the wait's status returned.
    let sync = f.call(Service::NotifyChangeKey, &[k, ev, 0, 0, iosb4, 1, 0, 0, 0, 0]);
    assert_eq!(sync, STATUS_TIMEOUT);
    assert_eq!(f.sys.registry.pending_requests(), 0);
    let apc = f.call(Service::NotifyChangeKey, &[k, ev, 0x1000, 0, iosb4, 1, 0, 0, 0, 1]);
    assert_eq!(apc, STATUS_NOT_IMPLEMENTED);
    assert_eq!(notify(&mut f, k, ev, iosb4, 0, false), STATUS_INVALID_PARAMETER);
    let (_, ro) = open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Watched", KEY_QUERY_VALUE);
    assert_eq!(notify(&mut f, ro, ev, iosb4, 1, false), STATUS_ACCESS_DENIED);
    // No event: the status block alone tells. The handle's filter is still
    // the first call's (REG_NOTIFY_CHANGE_NAME).
    let iosb5 = f.alloc(16);
    f.mem.write_bytes(iosb5, &[0xAA; 16]).unwrap();
    assert_eq!(notify(&mut f, k, 0, iosb5, REG_NOTIFY_CHANGE_LAST_SET, false), STATUS_PENDING);
    assert_eq!(set_value(&mut f, k, "x", REG_SZ, &sz("y")), STATUS_SUCCESS);
    assert_eq!(f.u32_at(iosb5), 0xAAAA_AAAA);
    assert_eq!(create_key(&mut f, k, "Sub2", KEY_READ, 0).0, STATUS_SUCCESS);
    assert_eq!(f.u32_at(iosb5), STATUS_SUCCESS.0);
    // The event's references went with the requests.
    close(&mut f, ev);
    assert_eq!(f.sys.registry.pending_requests(), 0);
}

#[test]
fn key_information_rules() {
    let mut f = Fake::new();
    let (_, k, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Info", KEY_ALL_ACCESS, 0);
    let (_, ro, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Info", KEY_ENUMERATE_SUB_KEYS, 0);
    // KeyNameInformation needs no access; the others KEY_QUERY_VALUE.
    assert_eq!(key_name(&mut f, ro), "\\REGISTRY\\MACHINE\\SOFTWARE\\Info");
    let buf = f.alloc(64);
    let ret = f.alloc(4);
    assert_eq!(f.call(Service::QueryKey, &[ro, u64::from(KEY_BASIC_INFORMATION), buf, 64, ret]), STATUS_ACCESS_DENIED);
    assert_eq!(f.call(Service::QueryKey, &[k, 5, buf, 64, ret]), STATUS_INVALID_PARAMETER);
    // Last-write time through KeyWriteTimeInformation.
    let t = f.put(&0x0123_4567_89AB_CDEFu64.to_le_bytes());
    assert_eq!(f.call(Service::SetInformationKey, &[k, 0, t, 8]), STATUS_SUCCESS);
    assert_eq!(f.call(Service::SetInformationKey, &[k, 0, t, 4]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(f.call(Service::SetInformationKey, &[k, 1, t, 4]), STATUS_INVALID_INFO_CLASS);
    assert_eq!(f.call(Service::QueryKey, &[k, u64::from(KEY_BASIC_INFORMATION), buf, 64, ret]), STATUS_SUCCESS);
    assert_eq!(f.u64_at(buf), 0x0123_4567_89AB_CDEF);
    // The Align64 classes want an aligned buffer.
    let n = f.unicode("x");
    let s = f.call(Service::QueryValueKey, &[k, n, u64::from(KEY_VALUE_PARTIAL_INFORMATION_ALIGN64), buf + 4, 32, ret]);
    assert_eq!(s, STATUS_DATATYPE_MISALIGNMENT);
    assert_eq!(set_value(&mut f, k, "x", REG_DWORD, &[9, 0, 0, 0]), STATUS_SUCCESS);
    let s = f.call(Service::QueryValueKey, &[k, n, u64::from(KEY_VALUE_PARTIAL_INFORMATION_ALIGN64), buf, 32, ret]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(f.bytes_at(buf, 12), [4, 0, 0, 0, 4, 0, 0, 0, 9, 0, 0, 0]);
    // A deleted key: everything but close fails.
    assert_eq!(f.call(Service::DeleteKey, &[k]), STATUS_SUCCESS);
    assert_eq!(f.call(Service::QueryKey, &[ro, u64::from(KEY_NAME_INFORMATION), buf, 64, ret]), STATUS_KEY_DELETED);
    assert_eq!(f.call(Service::FlushKey, &[k]), STATUS_KEY_DELETED);
    close(&mut f, k);
    close(&mut f, ro);
}

#[test]
fn flush_and_reload_keeps_non_volatile_keys_only() {
    let mut f = Fake::new();
    let (_, k, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Kept", KEY_ALL_ACCESS, 0);
    assert_eq!(set_value(&mut f, k, "v", REG_SZ, &sz("kept")), STATUS_SUCCESS);
    let (_, v, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Lost", KEY_ALL_ACCESS, REG_OPTION_VOLATILE);
    let sw = f.sys.registry.reg.hive_by_file("SOFTWARE").unwrap();
    assert!(f.sys.registry.reg.hive(sw).dirty);
    assert_eq!(f.call(Service::FlushKey, &[k]), STATUS_SUCCESS);
    assert!(!f.sys.registry.reg.hive(sw).dirty);
    assert_eq!(f.call(Service::FlushKey, &[v]), STATUS_SUCCESS, "a volatile key's hive is clean: nothing to do");

    // Save every hive and boot a new system from the files.
    let reg = &mut f.sys.registry.reg;
    let files: std::collections::BTreeMap<String, Vec<u8>> = (0..reg.hives().len() as u32)
        .map(poly_registry::HiveId)
        .map(|h| (reg.hive(h).file.clone(), reg.save(h)))
        .collect();
    let mut g = Fake::bare();
    g.sys = NtSystem::with_registry(NtSystem::default_registry(0, &mut |n: &str| files.get(n).cloned()));
    crate::exec::start_process(&mut g.sys, &mut g.proc, &mut g.mem, &test_exe(false), &fake_ntdll(), None, TID)
        .expect("starts");
    let k2 = open_ok(&mut g, "\\Registry\\Machine\\SOFTWARE\\Kept");
    let (s, _, b) = query_value(&mut g, k2, "V", KEY_VALUE_PARTIAL_INFORMATION, 64);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(&b[12..22], &sz("kept")[..]);
    assert_eq!(open_key(&mut g, 0, "\\Registry\\Machine\\SOFTWARE\\Lost", KEY_READ).0, STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(open_key(&mut g, 0, "\\Registry\\Machine\\SYSTEM\\CurrentControlSet", KEY_READ).0, STATUS_SUCCESS);
}

#[test]
fn default_user_hives() {
    let mut f = Fake::new();
    let me = alloc::format!("\\Registry\\User\\{}\\Volatile Environment", poly_registry::defaults::POLY_USER_SID);
    let h = open_ok(&mut f, &me);
    let (s, _, b) = query_value(&mut f, h, "USERNAME", KEY_VALUE_PARTIAL_INFORMATION, 64);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(&b[12..22], &sz("poly")[..]);
    assert_eq!(open_key(&mut f, 0, "\\Registry\\User\\.DEFAULT\\Environment", KEY_READ).0, STATUS_SUCCESS);
    let env = open_ok(&mut f, "\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment");
    let (s, _, b) = query_value(&mut f, env, "Path", KEY_VALUE_BASIC_INFORMATION, 64);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), REG_EXPAND_SZ);
}

// ----- the registry views of WOW64 (ADR 0186, stage 2) -------------------------------------------

/// The names of the subkeys of `h`, by `NtEnumerateKey(KeyBasicInformation)`.
fn subkeys(f: &mut Fake, h: u64) -> Vec<String> {
    let mut names = Vec::new();
    for i in 0.. {
        let buf = f.alloc(512);
        let ret = f.alloc(4);
        let s = f.call(Service::EnumerateKey, &[h, i, u64::from(KEY_BASIC_INFORMATION), buf, 512, ret]);
        if s != STATUS_SUCCESS {
            assert_eq!(s, STATUS_NO_MORE_ENTRIES);
            break;
        }
        // KEY_BASIC_INFORMATION: LastWriteTime 0, TitleIndex 8, NameLength 12, Name 16.
        let len = f.u32_at(buf + 12) as usize;
        names.push(utf16(&f.bytes_at(buf + 16, len)));
    }
    names
}

#[test]
fn the_default_software_hive_has_the_wow6432node_keys_of_windows() {
    let mut f = Fake::new();
    // `RegistryDefaults.Wow6432Node.yaml` `software_64`, `classes_wow`.
    let node = open_ok(&mut f, "\\Registry\\Machine\\SOFTWARE\\WOW6432Node");
    assert_eq!(subkeys(&mut f, node), ["Classes", "Clients", "Microsoft", "Policies", "RegisteredApplications"]);
    let classes_node = open_ok(&mut f, "\\Registry\\Machine\\SOFTWARE\\Classes\\WOW6432Node");
    let mut names = subkeys(&mut f, classes_node);
    names.sort();
    assert_eq!(names, ["AppID", "CLSID", "DirectShow", "Interface", "Media Type", "MediaFoundation", "TypeLib"]);
    // A 64-bit process lists it below HKLM\SOFTWARE.
    let sw = open_ok(&mut f, SOFTWARE);
    assert!(subkeys(&mut f, sw).iter().any(|n| n == "WOW6432Node"));
}

/// `RegistryDefaults.ClassesRoots.yaml` `hkcu_clsid`: a 32-bit program's create below the user's
/// `Classes\CLSID` makes the user's `Classes\WOW6432Node` it goes through; and
/// `one_key_64`: a 64-bit caller's create with `KEY_WOW64_64KEY` below `Classes\WOW6432Node`
/// opened with it stays there.
#[test]
fn view_nodes_on_create() {
    let mut f = Fake::new();
    let user_classes =
        alloc::format!("\\Registry\\User\\{}\\Software\\Classes", poly_registry::defaults::POLY_USER_SID);
    assert_eq!(
        open_key(&mut f, 0, &alloc::format!("{user_classes}\\WOW6432Node"), KEY_READ).0,
        STATUS_OBJECT_NAME_NOT_FOUND
    );
    f.proc.wow64 = true;
    let (s, h, _) = create_key(&mut f, 0, &alloc::format!("{user_classes}\\CLSID"), KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert!(key_name(&mut f, h).ends_with("\\Software\\Classes\\WOW6432Node\\CLSID"));
    f.proc.wow64 = false;
    let (s, node) =
        open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Classes\\Wow6432Node", KEY_ALL_ACCESS | KEY_WOW64_64KEY);
    assert_eq!(s, STATUS_SUCCESS);
    let (s, h, _) = create_key(&mut f, node, "Marker", KEY_ALL_ACCESS | KEY_WOW64_64KEY, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, h), "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\WOW6432Node\\Marker");
}

/// `RegistryDefaults.Wow6432Node.yaml` `one_key_64`, `software_32`: the physical registry's names
/// for one key (links), and the 32-bit view's own reading of a leftover `Wow6432Node`.
#[test]
fn the_wow6432node_names_of_one_key() {
    let mut f = Fake::new();
    let name = |f: &mut Fake, path: &str| {
        let (s, h) = open_key(f, 0, path, KEY_READ);
        (s == STATUS_SUCCESS).then(|| key_name(f, h))
    };
    let classes_node = "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\WOW6432Node";
    // 64-bit view: Wow6432Node\Classes is Classes\Wow6432Node; its TypeLib and AppID are the shared ones.
    let p = "\\Registry\\Machine\\SOFTWARE\\Wow6432Node\\Classes\\CLSID";
    assert_eq!(name(&mut f, p).as_deref(), Some(alloc::format!("{classes_node}\\CLSID").as_str()));
    let p = "\\Registry\\Machine\\SOFTWARE\\Classes\\Wow6432Node\\TypeLib";
    assert_eq!(name(&mut f, p).as_deref(), Some("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\TypeLib"));
    let p = "\\Registry\\Machine\\SOFTWARE\\Wow6432Node\\Classes\\AppID";
    assert_eq!(name(&mut f, p).as_deref(), Some("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\AppID"));
    assert_eq!(name(&mut f, "\\Registry\\Machine\\SOFTWARE\\Wow6432Node\\Wow6432Node"), None);
    // 32-bit view: SOFTWARE\Wow6432Node is SOFTWARE, so its Wow6432Node child does not exist.
    f.proc.wow64 = true;
    assert_eq!(name(&mut f, "\\Registry\\Machine\\SOFTWARE\\Wow6432Node\\Wow6432Node"), None);
    let p = "\\Registry\\Machine\\SOFTWARE\\Classes\\Wow6432Node\\Wow6432Node";
    assert_eq!(name(&mut f, p).as_deref(), Some(classes_node));
    // Its CurrentVersion has the (x86) folders.
    let p = "\\Registry\\Machine\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion";
    assert_eq!(
        name(&mut f, p).as_deref(),
        Some("\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion")
    );
}

#[test]
fn a_32_bit_process_opens_and_creates_software_keys_under_wow6432node() {
    let mut f = Fake::new();
    f.proc.wow64 = true;
    // The 32-bit process creates HKLM\SOFTWARE\Vendor ...
    let (s, h, d) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Vendor", KEY_ALL_ACCESS, 0);
    assert_eq!((s, d), (STATUS_SUCCESS, REG_CREATED_NEW_KEY));
    assert_eq!(key_name(&mut f, h), "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node\\Vendor");
    // ... and finds it again in its own view.
    assert_eq!(open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Vendor", KEY_READ).0, STATUS_SUCCESS);
    // With KEY_WOW64_64KEY it sees the native view, where there is no such key.
    assert_eq!(
        open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Vendor", KEY_READ | KEY_WOW64_64KEY).0,
        STATUS_OBJECT_NAME_NOT_FOUND
    );
    let (s, n) = open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\WOW6432Node\\Vendor", KEY_READ);
    assert_eq!(s, STATUS_SUCCESS, "the physical key is below the node");
    assert_eq!(key_name(&mut f, n), "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node\\Vendor");
    // A 32-bit process sees no Wow6432Node below HKLM\SOFTWARE (it is HKLM\SOFTWARE).
    let sw = open_ok(&mut f, SOFTWARE);
    assert_eq!(key_name(&mut f, sw), "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node");
    assert!(!subkeys(&mut f, sw).iter().any(|n| n == "WOW6432Node"));
    assert!(subkeys(&mut f, sw).iter().any(|n| n == "Vendor"));
}

#[test]
fn a_64_bit_process_does_not_see_the_32_bit_keys_unless_it_asks() {
    let mut f = Fake::new();
    f.proc.wow64 = true;
    create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Only32", KEY_ALL_ACCESS, 0);
    f.proc.wow64 = false;
    assert_eq!(open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Only32", KEY_READ).0, STATUS_OBJECT_NAME_NOT_FOUND);
    let (s, h) = open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Only32", KEY_READ | KEY_WOW64_32KEY);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, h), "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node\\Only32");
    // The mask bits are no rights: creating with them works in a 64-bit process, too.
    let (s, h, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Made32", KEY_ALL_ACCESS | KEY_WOW64_32KEY, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, h), "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node\\Made32");
    assert_eq!(open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Made32", KEY_READ).0, STATUS_OBJECT_NAME_NOT_FOUND);
}

#[test]
fn a_relative_open_below_hklm_software_is_redirected_for_a_32_bit_process() {
    let mut f = Fake::new();
    // kernelbase opens HKLM (\REGISTRY\MACHINE) as the root and names the rest relative to it.
    let machine = open_ok(&mut f, "\\Registry\\Machine");
    f.proc.wow64 = true;
    let (s, h, _) = create_key(&mut f, machine, "SOFTWARE\\Rel", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, h), "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node\\Rel");
    // A handle to HKLM\SOFTWARE (the 32-bit view) and a relative create below it.
    let (s, sw) = open_key(&mut f, machine, "SOFTWARE", KEY_ALL_ACCESS);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, sw), "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node");
    let (s, h, _) = create_key(&mut f, sw, "Below", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, h), "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node\\Below");
    // The native view of the same relative name.
    let (s, native) = open_key(&mut f, machine, "SOFTWARE", KEY_ALL_ACCESS | KEY_WOW64_64KEY);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, native), "\\REGISTRY\\MACHINE\\SOFTWARE");
    // A key that is not under SOFTWARE is the same for both.
    let (s, sys) = open_key(&mut f, machine, "SYSTEM", KEY_READ);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, sys), "\\REGISTRY\\MACHINE\\SYSTEM");
}

/// `RegCreateKeyExW.yaml` `inheritance`, `level_by_level`, `below_software`, `named_node`,
/// `empty_subkey`: a key made below an open key with no view bit is in that key's view, and the
/// view is part of the open key's physical place.
#[test]
fn a_32_bit_process_makes_keys_in_the_view_of_the_open_key() {
    let mut f = Fake::new();
    f.proc.wow64 = true;
    let machine = open_ok(&mut f, "\\Registry\\Machine");
    let n = |f: &mut Fake, root: u64, name: &str, access: u32| {
        let (s, h, _) = create_key(f, root, name, access, 0);
        assert_eq!(s, STATUS_SUCCESS, "{name}");
        (h, key_name(f, h))
    };
    let classes_n = "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes";
    let clsid_x = "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\WOW6432Node\\CLSID";
    let clsid_n = "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\CLSID";
    // Classes opened with 64: CLSID and its subkey with no bit are native; with 32, the 32-bit ones.
    let (classes64, name) = n(&mut f, machine, "SOFTWARE\\Classes", KEY_ALL_ACCESS | KEY_WOW64_64KEY);
    assert_eq!(name, classes_n);
    let (clsid64, name) = n(&mut f, classes64, "CLSID", KEY_ALL_ACCESS);
    assert_eq!(name, clsid_n);
    assert_eq!(n(&mut f, clsid64, "{G}", KEY_ALL_ACCESS).1, alloc::format!("{clsid_n}\\{{G}}"));
    assert_eq!(n(&mut f, clsid64, "{G}", KEY_ALL_ACCESS | KEY_WOW64_32KEY).1, alloc::format!("{clsid_x}\\{{G}}"));
    // The open key itself again (kernelbase opens an empty name) keeps its view.
    let (s, again) = open_key(&mut f, clsid64, "", KEY_ALL_ACCESS);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, again), clsid_n);
    assert_eq!(n(&mut f, again, "{H}", KEY_ALL_ACCESS).1, alloc::format!("{clsid_n}\\{{H}}"));
    // Classes opened with no bit (shared): CLSID is the 32-bit one, and a subkey of that stays
    // there even with 64; CLSID with 64 is the native one.
    let (classes, name) = n(&mut f, machine, "SOFTWARE\\Classes", KEY_ALL_ACCESS);
    assert_eq!(name, classes_n);
    let (clsid, name) = n(&mut f, classes, "CLSID", KEY_ALL_ACCESS);
    assert_eq!(name, clsid_x);
    assert_eq!(n(&mut f, clsid, "{G}", KEY_ALL_ACCESS | KEY_WOW64_64KEY).1, alloc::format!("{clsid_x}\\{{G}}"));
    assert_eq!(n(&mut f, classes, "CLSID", KEY_ALL_ACCESS | KEY_WOW64_64KEY).1, clsid_n);
    // A path that names the node is the physical key, also below a native handle.
    assert_eq!(n(&mut f, classes64, "WOW6432Node\\CLSID", KEY_ALL_ACCESS).1, clsid_x);
    // Software opened with no bit is Software\WOW6432Node; Classes below it is the shared
    // Classes, and CLSID below that the 32-bit one (Google's updater makes its class this way).
    let (sw, name) = n(&mut f, machine, "SOFTWARE", KEY_ALL_ACCESS);
    assert_eq!(name, "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node");
    let (cl, name) = n(&mut f, sw, "Classes", KEY_ALL_ACCESS);
    assert_eq!(name, classes_n);
    assert_eq!(n(&mut f, cl, "CLSID", KEY_ALL_ACCESS).1, clsid_x);
    // AppID is shared: the walk and the one-call path reach the same key (`shared_keys`).
    let (appid, _) = n(&mut f, cl, "AppID", KEY_ALL_ACCESS);
    let (_, name) = n(&mut f, appid, "{A}", KEY_ALL_ACCESS);
    assert_eq!(name, "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\AppID\\{A}");
    let (s, h) = open_key(&mut f, machine, "SOFTWARE\\Classes\\AppID\\{A}", KEY_READ);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, h), name);
    // A vendor key below it stays in the 32-bit view with 64.
    let vendor = n(&mut f, sw, "Vendor", KEY_ALL_ACCESS | KEY_WOW64_64KEY).1;
    assert_eq!(vendor, "\\REGISTRY\\MACHINE\\SOFTWARE\\WOW6432Node\\Vendor");
    // Software opened with 64: a vendor key with no bit is native, with 32 the 32-bit one.
    let (sw64, _) = n(&mut f, machine, "SOFTWARE", KEY_ALL_ACCESS | KEY_WOW64_64KEY);
    assert_eq!(n(&mut f, sw64, "Vendor", KEY_ALL_ACCESS).1, "\\REGISTRY\\MACHINE\\SOFTWARE\\Vendor");
    assert_eq!(n(&mut f, sw64, "Vendor", KEY_ALL_ACCESS | KEY_WOW64_32KEY).1, vendor);
}

/// The default COM marshalling registrations exist in the 32-bit view too (ADR 0186 addendum):
/// a 32-bit process finds `IDispatch`'s proxy/stub class and `PSOAInterface`'s server.
#[test]
fn a_32_bit_process_finds_the_default_com_registrations() {
    let mut f = Fake::new();
    f.proc.wow64 = true;
    let classes = "\\Registry\\Machine\\SOFTWARE\\Classes";
    for (path, physical) in [
        (
            "Interface\\{00020400-0000-0000-C000-000000000046}\\ProxyStubClsid32",
            "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\WOW6432Node\\Interface\\{00020400-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        ),
        (
            "CLSID\\{00020424-0000-0000-C000-000000000046}\\InprocServer32",
            "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\WOW6432Node\\CLSID\\{00020424-0000-0000-C000-000000000046}\\InprocServer32",
        ),
    ] {
        let (s, h) = open_key(&mut f, 0, &alloc::format!("{classes}\\{path}"), KEY_READ);
        assert_eq!(s, STATUS_SUCCESS, "{path}");
        assert_eq!(key_name(&mut f, h), physical);
    }
}

#[test]
fn shared_keys_are_the_same_in_both_views() {
    let mut f = Fake::new();
    let policies = "\\Registry\\Machine\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies";
    // A 64-bit process makes a shared key; the 32-bit one finds it, and the other way round.
    let (s, _, _) = create_key(&mut f, 0, policies, KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    f.proc.wow64 = true;
    let (s, h) = open_key(&mut f, 0, policies, KEY_READ);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(key_name(&mut f, h), "\\REGISTRY\\MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies");
    // `Classes` is shared; its CLSID subkey is not.
    let (s, _, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Classes\\.poly", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    f.proc.wow64 = false;
    assert_eq!(open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Classes\\.poly", KEY_READ).0, STATUS_SUCCESS);
    let (s, _, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Classes", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
}

#[test]
fn deleting_a_key_deletes_the_one_the_handle_names() {
    let mut f = Fake::new();
    f.proc.wow64 = true;
    let (s, h, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Gone", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(f.call(Service::DeleteKey, &[h]), STATUS_SUCCESS);
    close(&mut f, h);
    assert_eq!(open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Gone", KEY_READ).0, STATUS_OBJECT_NAME_NOT_FOUND);
    // RegDeleteKeyEx opens with DELETE and the view flag and then deletes.
    f.proc.wow64 = false;
    let (s, h, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Gone64", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    close(&mut f, h);
    f.proc.wow64 = true;
    let (s, h) = open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\Gone64", DELETE | KEY_WOW64_64KEY);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(f.call(Service::DeleteKey, &[h]), STATUS_SUCCESS);
}

fn query_sd(f: &mut Fake, h: u64, info: u32) -> (NtStatus, Option<(u16, poly_security::sd::Parts)>) {
    let buf = f.alloc(1024);
    let need = f.alloc(4);
    let s = f.call(Service::QuerySecurityObject, &[h, u64::from(info), buf, 1024, need]);
    if s != STATUS_SUCCESS {
        return (s, None);
    }
    let n = f.u32_at(need) as usize;
    let bytes = f.bytes_at(buf, n);
    let wire_ctl = u16::from_le_bytes([bytes[2], bytes[3]]);
    (s, poly_security::sd::Parts::from_relative(&bytes).map(|p| (wire_ctl, p)))
}

fn set_sd(f: &mut Fake, h: u64, info: u32, parts: &poly_security::sd::Parts) -> NtStatus {
    let rel = parts.to_relative();
    let p = f.put(&rel);
    f.call(Service::SetSecurityObject, &[h, u64::from(info), p])
}

#[test]
fn registry_default_and_new_key_security_descriptors() {
    use poly_security::{acl, sd, sddl, sid};
    let mut f = Fake::new();
    let u = poly_registry::defaults::POLY_USER_SID;

    // Predefined / seeded roots: control, owner, ACE count.
    for (path, want_ctl, want_owner, want_aces) in [
        ("\\Registry\\Machine", 0x8004u16, "S-1-5-18", 6usize),
        ("\\Registry\\User", 0x8004, "S-1-5-18", 6),
        ("\\Registry\\Machine\\SYSTEM\\CurrentControlSet\\Hardware Profiles\\Current", 0x8004, "S-1-5-18", 6),
        ("\\Registry\\Machine\\SOFTWARE", 0x9404, "S-1-5-32-544", 6),
        ("\\Registry\\Machine\\SOFTWARE\\WOW6432Node", 0x8404, "S-1-5-18", 6),
        ("\\Registry\\Machine\\SOFTWARE\\Classes", 0x9404, "S-1-5-18", 12),
        ("\\Registry\\Machine\\SOFTWARE\\Classes\\CLSID", 0x9004, u, 6),
        ("\\Registry\\User\\.DEFAULT", 0x9404, "S-1-5-32-544", 6),
        (&alloc::format!("\\Registry\\User\\{u}"), 0x9004, "S-1-5-32-544", 6),
    ] {
        let (s, h) = open_key(&mut f, 0, path, READ_CONTROL);
        assert_eq!(s, STATUS_SUCCESS, "{path}");
        let (qs, res) = query_sd(&mut f, h, sd::OWNER_SECURITY_INFORMATION | sd::DACL_SECURITY_INFORMATION);
        assert_eq!(qs, STATUS_SUCCESS, "{path}");
        let (ctl, p) = res.unwrap();
        assert_eq!(ctl, want_ctl, "{path} control");
        let owner = sddl::to_string(&sid::Sid::from_bytes(p.owner.as_deref().unwrap()).unwrap());
        assert_eq!(owner, want_owner, "{path} owner");
        assert_eq!(acl::aces_in(p.dacl.as_deref().unwrap()).len(), want_aces, "{path} aces");
        close(&mut f, h);
    }

    // New 64-bit key below HKLM\SOFTWARE: BA owner, <machine>-513 group, 0x8404, 6 inherited ACEs (CO 0x1a).
    let (s, h64, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\SecTest64", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (ctl, p) = query_sd(&mut f, h64, 7).1.unwrap();
    assert_eq!(ctl, 0x8404);
    assert_eq!(sddl::to_string(&sid::Sid::from_bytes(p.owner.as_deref().unwrap()).unwrap()), "S-1-5-32-544");
    assert_eq!(
        sddl::to_string(&sid::Sid::from_bytes(p.group.as_deref().unwrap()).unwrap()),
        "S-1-5-21-520193847-884726105-1936024571-513"
    );
    let aces = acl::aces_in(p.dacl.as_deref().unwrap());
    assert_eq!(aces.len(), 6);
    assert_eq!(aces[0].flags, 0x12);
    assert_eq!(aces[3].flags, 0x1a);
    // 64-bit key LABEL returns 0x8000; SACL needs SeSecurityPrivilege enabled (`Token::poly_user`).
    assert_eq!(query_sd(&mut f, h64, sd::LABEL_SECURITY_INFORMATION).1.unwrap().0, 0x8000);
    let (tid_admin, _) = f.sys.objects.new_token(poly_security::token::Token::poly_user(), TID).unwrap();
    f.sys.objects.set_process_token(PID, tid_admin).unwrap();
    let (so, hs64) = open_key(&mut f, h64, "", ACCESS_SYSTEM_SECURITY | READ_CONTROL);
    assert_eq!(so, STATUS_SUCCESS, "open hs64");
    assert_eq!(query_sd(&mut f, hs64, sd::SACL_SECURITY_INFORMATION).1.unwrap().0, 0x8000);
    assert_eq!(query_sd(&mut f, hs64, sd::LABEL_SECURITY_INFORMATION).1.unwrap().0, 0x8000);
    close(&mut f, hs64);
    close(&mut f, h64);

    // New 32-bit key below HKLM\SOFTWARE (WOW64 -> Wow6432Node): SACL and LABEL return 0x8800.
    f.proc.wow64 = true;
    let (s, h32, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\SecTest32", KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(query_sd(&mut f, h32, sd::DACL_SECURITY_INFORMATION).1.unwrap().0, 0x8404);
    let (so32, hs32) = open_key(&mut f, h32, "", ACCESS_SYSTEM_SECURITY | READ_CONTROL);
    assert_eq!(so32, STATUS_SUCCESS, "open hs32");
    assert_eq!(query_sd(&mut f, hs32, sd::SACL_SECURITY_INFORMATION).1.unwrap().0, 0x8800);
    assert_eq!(query_sd(&mut f, hs32, sd::LABEL_SECURITY_INFORMATION).1.unwrap().0, 0x8800);
    close(&mut f, hs32);
    close(&mut f, h32);
    f.proc.wow64 = false;

    // New key below HKCU\Software: control 0x8004, owner BA, 4 inherited ACEs.
    let (s, hu, _) =
        create_key(&mut f, 0, &alloc::format!("\\Registry\\User\\{u}\\Software\\SecTestU"), KEY_ALL_ACCESS, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (ctl_u, pu) = query_sd(&mut f, hu, sd::OWNER_SECURITY_INFORMATION | sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_u, 0x8004);
    assert_eq!(sddl::to_string(&sid::Sid::from_bytes(pu.owner.as_deref().unwrap()).unwrap()), "S-1-5-32-544");
    assert_eq!(acl::aces_in(pu.dacl.as_deref().unwrap()).len(), 4);
    close(&mut f, hu);
}

#[test]
fn registry_set_security_inheritance_protection_and_verbatim() {
    use poly_security::{acl, sd, sdops, sdstr};
    let mut f = Fake::new();
    let (_, hb, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\SecBase", KEY_ALL_ACCESS, 0);
    let (_, ha, _) = create_key(&mut f, hb, "A", KEY_ALL_ACCESS, 0);
    let (_, hab, _) = create_key(&mut f, ha, "B", KEY_ALL_ACCESS, 0);

    let parse_dacl = |s: &str| {
        sd::Parts::from_relative(&sdstr::parse(&s.encode_utf16().collect::<Vec<_>>()).unwrap()).unwrap().dacl.unwrap()
    };

    // 1. SetSecurityInfo-style (SE_DACL_AUTO_INHERIT_REQ): canonical order (explicit first) & propagation.
    let mut aces =
        acl::aces_in(query_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION).1.unwrap().1.dacl.as_deref().unwrap());
    aces.extend(acl::aces_in(&parse_dacl("D:(A;CI;KR;;;AU)")));
    let mut new_p = sd::Parts {
        revision: sd::SECURITY_DESCRIPTOR_REVISION,
        control: sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERIT_REQ,
        dacl: Some(sdops::acl_of(acl::ACL_REVISION, &aces)),
        ..sd::Parts::default()
    };
    assert_eq!(set_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION, &new_p), STATUS_SUCCESS);
    let (ctl_b, pb) = query_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_b, 0x8404);
    let ab = acl::aces_in(pb.dacl.as_deref().unwrap());
    assert_eq!(ab.len(), 7);
    assert_eq!(ab[0].flags, 0x02);
    for h_sub in [ha, hab] {
        let (ctl_s, ps) = query_sd(&mut f, h_sub, sd::DACL_SECURITY_INFORMATION).1.unwrap();
        assert_eq!(ctl_s, 0x8404);
        let asub = acl::aces_in(ps.dacl.as_deref().unwrap());
        assert_eq!(asub.len(), 7);
        assert_eq!(asub[0].flags, 0x12);
    }

    // 2. PROTECTED_DACL_SECURITY_INFORMATION: inherited ACEs converted to explicit (0x02 / 0x0a), control 0x9404.
    let mut aces2 = acl::aces_in(pb.dacl.as_deref().unwrap());
    aces2.extend(acl::aces_in(&parse_dacl("D:(A;CI;0x1;;;IU)")));
    new_p.dacl = Some(sdops::acl_of(acl::ACL_REVISION, &aces2));
    assert_eq!(
        set_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION | sd::PROTECTED_DACL_SECURITY_INFORMATION, &new_p),
        STATUS_SUCCESS
    );
    let (ctl_b2, pb2) = query_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_b2, 0x9404);
    let ab2 = acl::aces_in(pb2.dacl.as_deref().unwrap());
    assert_eq!(ab2.len(), 8);
    assert!(ab2.iter().all(|a| a.flags & 0x10 == 0));
    assert_eq!(
        acl::aces_in(query_sd(&mut f, ha, sd::DACL_SECURITY_INFORMATION).1.unwrap().1.dacl.as_deref().unwrap()).len(),
        8
    );

    // 3. UNPROTECTED_DACL_SECURITY_INFORMATION: 9 explicit + 6 re-inherited = 15, child deduplicates to 8!
    let mut aces3 = acl::aces_in(pb2.dacl.as_deref().unwrap());
    aces3.extend(acl::aces_in(&parse_dacl("D:(A;CI;0x1;;;IU)")));
    new_p.dacl = Some(sdops::acl_of(acl::ACL_REVISION, &aces3));
    assert_eq!(
        set_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION | sd::UNPROTECTED_DACL_SECURITY_INFORMATION, &new_p),
        STATUS_SUCCESS
    );
    let (ctl_b3, pb3) = query_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_b3, 0x8404);
    assert_eq!(acl::aces_in(pb3.dacl.as_deref().unwrap()).len(), 15);
    assert_eq!(
        acl::aces_in(query_sd(&mut f, ha, sd::DACL_SECURITY_INFORMATION).1.unwrap().1.dacl.as_deref().unwrap()).len(),
        8
    );

    // 4. NULL DACL (SetSecurityInfo): 0x9404, child has 0 ACEs (0x8404); empty DACL afterwards stays 0x9404.
    let null_p = sd::Parts {
        revision: sd::SECURITY_DESCRIPTOR_REVISION,
        control: sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERIT_REQ | sd::SE_DACL_PROTECTED,
        dacl: None,
        ..sd::Parts::default()
    };
    assert_eq!(set_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION, &null_p), STATUS_SUCCESS);
    let (ctl_null, pnull) = query_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_null, 0x9404);
    assert!(pnull.dacl.is_none());
    let (ctl_cnull, cnull) = query_sd(&mut f, ha, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_cnull, 0x8404);
    assert_eq!(acl::aces_in(cnull.dacl.as_deref().unwrap()).len(), 0);

    let empty_p = sd::Parts {
        revision: sd::SECURITY_DESCRIPTOR_REVISION,
        control: sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERIT_REQ,
        dacl: Some(sdops::acl_of(acl::ACL_REVISION, &[])),
        ..sd::Parts::default()
    };
    assert_eq!(set_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION, &empty_p), STATUS_SUCCESS);
    let (ctl_empty, pempty) = query_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_empty, 0x9404);
    assert_eq!(acl::aces_in(pempty.dacl.as_deref().unwrap()).len(), 0);

    // 5. Delete and recreate resets to fresh inherited SD; then test RegSetKeySecurity verbatim non-propagation.
    assert_eq!(f.call(Service::DeleteKey, &[hab]), STATUS_SUCCESS);
    close(&mut f, hab);
    assert_eq!(f.call(Service::DeleteKey, &[ha]), STATUS_SUCCESS);
    close(&mut f, ha);
    assert_eq!(f.call(Service::DeleteKey, &[hb]), STATUS_SUCCESS);
    close(&mut f, hb);

    let (_, hb, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\SecBase", KEY_ALL_ACCESS, 0);
    let (_, ha, _) = create_key(&mut f, hb, "A", KEY_ALL_ACCESS, 0);
    let mut raw_aces =
        acl::aces_in(query_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION).1.unwrap().1.dacl.as_deref().unwrap());
    raw_aces.extend(acl::aces_in(&parse_dacl("D:(A;CI;KR;;;AU)")));
    let verb = sd::Parts {
        revision: sd::SECURITY_DESCRIPTOR_REVISION,
        control: sd::SE_DACL_PRESENT,
        dacl: Some(sdops::acl_of(acl::ACL_REVISION, &raw_aces)),
        ..sd::Parts::default()
    };
    assert_eq!(set_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION, &verb), STATUS_SUCCESS);
    let (ctl_v, pv) = query_sd(&mut f, hb, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_v, 0x8004);
    let av = acl::aces_in(pv.dacl.as_deref().unwrap());
    assert_eq!(av.len(), 7);
    assert_eq!(av[6].flags, 0x02, "verbatim keeps appended ACE last");
    // Existing child was NOT modified by RegSetKeySecurity.
    let (ctl_ca, ca) = query_sd(&mut f, ha, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_ca, 0x8404);
    assert_eq!(acl::aces_in(ca.dacl.as_deref().unwrap()).len(), 6);

    // 1 explicit ACE (no inherited ACEs) via verbatim -> 0x9004.
    let one = sd::Parts {
        revision: sd::SECURITY_DESCRIPTOR_REVISION,
        control: sd::SE_DACL_PRESENT,
        dacl: Some(parse_dacl("D:(A;;KR;;;SY)")),
        ..sd::Parts::default()
    };
    assert_eq!(set_sd(&mut f, ha, sd::DACL_SECURITY_INFORMATION, &one), STATUS_SUCCESS);
    assert_eq!(query_sd(&mut f, ha, sd::DACL_SECURITY_INFORMATION).1.unwrap().0, 0x9004);

    // Verbatim NULL DACL -> 0x8004.
    let vnull =
        sd::Parts { revision: sd::SECURITY_DESCRIPTOR_REVISION, control: 0, dacl: None, ..sd::Parts::default() };
    assert_eq!(set_sd(&mut f, ha, sd::DACL_SECURITY_INFORMATION, &vnull), STATUS_SUCCESS);
    let (ctl_n, pn) = query_sd(&mut f, ha, sd::DACL_SECURITY_INFORMATION).1.unwrap();
    assert_eq!(ctl_n, 0x8004);
    assert!(pn.dacl.is_none());
    close(&mut f, ha);
    close(&mut f, hb);
}

#[test]
fn registry_key_reopen_owner_implied_and_non_owner_rights() {
    use poly_security::{sd, sdstr, sid};
    let mut f = Fake::new();
    let (_, hk, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\OwnTest", KEY_WRITE, 0);
    let (_, hall) = open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\OwnTest", KEY_ALL_ACCESS);
    // Replace DACL with protected SYSTEM-only ACE (owner BA is not in DACL).
    let sy_only = sd::Parts {
        revision: sd::SECURITY_DESCRIPTOR_REVISION,
        control: sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERIT_REQ | sd::SE_DACL_PROTECTED,
        dacl: sd::Parts::from_relative(&sdstr::parse(&"D:(A;;KA;;;SY)".encode_utf16().collect::<Vec<_>>()).unwrap())
            .unwrap()
            .dacl,
        ..sd::Parts::default()
    };
    assert_eq!(
        set_sd(&mut f, hall, sd::DACL_SECURITY_INFORMATION | sd::PROTECTED_DACL_SECURITY_INFORMATION, &sy_only),
        STATUS_SUCCESS
    );
    close(&mut f, hall);
    // New opens with KEY_WRITE fail, but READ_CONTROL and WRITE_DAC succeed by owner's implied rights.
    assert_eq!(open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\OwnTest", KEY_WRITE).0, STATUS_ACCESS_DENIED);
    let (s_wd, h_wd) = open_key(&mut f, hk, "", WRITE_DAC);
    assert_eq!(s_wd, STATUS_SUCCESS);
    close(&mut f, h_wd);
    // Original KEY_WRITE handle's granted access is unchanged (0x20006).
    let basic = f.alloc(64);
    let ret = f.alloc(4);
    assert_eq!(
        f.call(Service::QueryObject, &[hk, u64::from(OBJECT_BASIC_INFORMATION), basic, 64, ret]),
        STATUS_SUCCESS
    );
    assert_eq!(f.u32_at(basic + 4), KEY_WRITE);
    close(&mut f, hk);

    // Non-owner: transfer ownership to SYSTEM and give BA only DELETE|KEY_WRITE|KEY_READ (no WRITE_DAC).
    let (_, h, _) = create_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\NonOwnTest", KEY_ALL_ACCESS, 0);
    let set_owner = sd::Parts {
        revision: sd::SECURITY_DESCRIPTOR_REVISION,
        owner: Some(sid::nt(18).to_bytes()),
        ..sd::Parts::default()
    };
    assert_eq!(set_sd(&mut f, h, sd::OWNER_SECURITY_INFORMATION, &set_owner), STATUS_SUCCESS);
    let no_wd = sd::Parts {
        revision: sd::SECURITY_DESCRIPTOR_REVISION,
        control: sd::SE_DACL_PRESENT | sd::SE_DACL_AUTO_INHERIT_REQ | sd::SE_DACL_PROTECTED,
        dacl: sd::Parts::from_relative(
            &sdstr::parse(&"D:(A;;KA;;;SY)(A;;0x3001f;;;BA)".encode_utf16().collect::<Vec<_>>()).unwrap(),
        )
        .unwrap()
        .dacl,
        ..sd::Parts::default()
    };
    assert_eq!(
        set_sd(&mut f, h, sd::DACL_SECURITY_INFORMATION | sd::PROTECTED_DACL_SECURITY_INFORMATION, &no_wd),
        STATUS_SUCCESS
    );
    close(&mut f, h);

    let (s, hkw) = open_key(&mut f, 0, "\\Registry\\Machine\\SOFTWARE\\NonOwnTest", KEY_WRITE);
    assert_eq!(s, STATUS_SUCCESS);
    let (s_rc, h_rc) = open_key(&mut f, hkw, "", READ_CONTROL);
    assert_eq!(s_rc, STATUS_SUCCESS);
    close(&mut f, h_rc);
    assert_eq!(open_key(&mut f, hkw, "", WRITE_DAC).0, STATUS_ACCESS_DENIED);
    close(&mut f, hkw);
}
