use super::memory::{allocate, query};
use super::*;
use crate::consts::*;
use poly_ntobj::access::*;
use poly_ntobj::status::*;
use poly_ntobj::{NewObject, OBJ_CASE_INSENSITIVE, ObjectType};

const SECTION: &str = "\\BaseNamedObjects\\PolySection";

fn create(f: &mut Fake, name: &str, size: u64, protect: u32, attributes: u32, file: u64) -> (NtStatus, u64) {
    let out = f.alloc(8);
    let oa = if name.is_empty() { 0 } else { f.attrs(name, OBJ_CASE_INSENSITIVE) };
    let size = if size == 0 { 0 } else { f.put_u64(size) };
    let s = f.call(
        Service::CreateSection,
        &[out, u64::from(SECTION_ALL_ACCESS), oa, size, u64::from(protect), u64::from(attributes), file],
    );
    (s, f.u64_at(out))
}

/// `NtMapViewOfSection`: (status, base, size, offset).
fn map(f: &mut Fake, h: u64, base: u64, offset: Option<u64>, size: u64, protect: u32) -> (NtStatus, u64, u64, u64) {
    map_with(f, h, base, 0, offset, size, protect)
}

fn map_with(
    f: &mut Fake,
    h: u64,
    base: u64,
    commit: u64,
    offset: Option<u64>,
    size: u64,
    protect: u32,
) -> (NtStatus, u64, u64, u64) {
    let b = f.put_u64(base);
    let o = offset.map_or(0, |o| f.put_u64(o));
    let s = f.put_u64(size);
    let st =
        f.call(Service::MapViewOfSection, &[h, SELF, b, 0, commit, o, s, u64::from(VIEW_UNMAP), 0, u64::from(protect)]);
    (st, f.u64_at(b), f.u64_at(s), if o == 0 { 0 } else { f.u64_at(o) })
}

#[test]
fn a_pagefile_section_mapped_twice_shares_its_pages() {
    let mut f = Fake::new();
    let (s, h) = create(&mut f, "", 0x2000, PAGE_READWRITE, SEC_COMMIT, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (s, v1, n1, _) = map(&mut f, h, 0, None, 0, PAGE_READWRITE);
    assert_eq!((s, n1), (STATUS_SUCCESS, 0x2000));
    let (s, v2, _, _) = map(&mut f, h, 0, None, 0, PAGE_READWRITE);
    assert_eq!(s, STATUS_SUCCESS);
    assert_ne!(v1, v2);
    f.mem.write_bytes(v1 + 0x1008, b"shared").unwrap();
    assert_eq!(f.mem.bytes_at(v2 + 0x1008, 6), b"shared");
    let q = query(&mut f, v1);
    assert_eq!(
        (q.state, q.type_, q.protect, q.region_size, q.allocation_base),
        (MEM_COMMIT, MEM_MAPPED, PAGE_READWRITE, 0x2000, v1)
    );
    // A read-only view of the same section; a copy-on-write view keeps its writes.
    let (_, ro, _, _) = map(&mut f, h, 0, None, 0, PAGE_READONLY);
    assert!(f.mem.write_bytes(ro, b"x").is_err());
    let (_, cow, _, _) = map(&mut f, h, 0, None, 0, PAGE_WRITECOPY);
    f.mem.write_bytes(cow + 0x1008, b"COPIED").unwrap();
    assert_eq!(f.mem.bytes_at(v2 + 0x1008, 6), b"shared");
    // Unmap one by an address inside it.
    assert_eq!(f.call(Service::UnmapViewOfSection, &[SELF, v1 + 0x1234]), STATUS_SUCCESS);
    assert_eq!(query(&mut f, v1).state, MEM_FREE);
    assert_eq!(f.call(Service::UnmapViewOfSection, &[SELF, v1]), STATUS_NOT_MAPPED_VIEW);
    // Views keep the section after its handle closes.
    let sections = f.sys.section_count();
    assert_eq!(f.call(Service::Close, &[h]), STATUS_SUCCESS);
    assert_eq!(f.sys.section_count(), sections);
    assert_eq!(f.mem.bytes_at(v2 + 0x1008, 6), b"shared");
    for v in [v2, ro, cow] {
        assert_eq!(f.call(Service::UnmapViewOfSection, &[SELF, v]), STATUS_SUCCESS);
    }
    assert_eq!(f.sys.section_count(), sections - 1);
    // Freeing memory of a view is not how views go.
    let (_, h) = create(&mut f, "", 0x1000, PAGE_READWRITE, SEC_COMMIT, 0);
    let (_, v, _, _) = map(&mut f, h, 0, None, 0, PAGE_READWRITE);
    let b = f.put_u64(v);
    let z = f.put_u64(0);
    assert_eq!(
        f.call(Service::FreeVirtualMemory, &[SELF, b, z, u64::from(MEM_RELEASE)]),
        STATUS_UNABLE_TO_DELETE_SECTION
    );
}

#[test]
fn named_sections_and_offsets() {
    let mut f = Fake::new();
    let (s, h) = create(&mut f, SECTION, 0x3_0000, PAGE_READWRITE, SEC_COMMIT, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let out = f.alloc(8);
    let oa = f.attrs(SECTION, OBJ_CASE_INSENSITIVE);
    assert_eq!(f.call(Service::OpenSection, &[out, u64::from(SECTION_MAP_READ), oa]), STATUS_SUCCESS);
    let ro = f.u64_at(out);
    let (_, rw_view, _, _) = map(&mut f, h, 0, None, 0, PAGE_READWRITE);
    f.mem.write_bytes(rw_view + 0x1_0010, b"at 64k").unwrap();
    // The offset is rounded down to 64 KiB and the size grows by the difference.
    let (s, v, n, o) = map(&mut f, ro, 0, Some(0x1_0010), 0x10, PAGE_READONLY);
    assert_eq!((s, n, o), (STATUS_SUCCESS, 0x1000, 0x1_0000));
    assert_eq!(f.mem.bytes_at(v + 0x10, 6), b"at 64k");
    // The read-only handle cannot map for writing.
    assert_eq!(map(&mut f, ro, 0, None, 0, PAGE_READWRITE).0, STATUS_ACCESS_DENIED);
    // Views past the end.
    assert_eq!(map(&mut f, h, 0, Some(0x2_0000), 0x2_0000, PAGE_READWRITE).0, STATUS_INVALID_VIEW_SIZE);
    assert_eq!(map(&mut f, h, 0, Some(0x3_0000), 0, PAGE_READWRITE).0, STATUS_INVALID_VIEW_SIZE);
    // At a given base.
    let (s, v, _, _) = map(&mut f, h, 0x6000_0000, None, 0x1000, PAGE_READWRITE);
    assert_eq!((s, v), (STATUS_SUCCESS, 0x6000_0000));
    assert_eq!(map(&mut f, h, 0x6000_0000, None, 0x1000, PAGE_READWRITE).0, STATUS_CONFLICTING_ADDRESSES);
    // The name is taken; the second section is dropped again.
    let backings = f.mem.live_sections();
    assert_eq!(create(&mut f, SECTION, 0x1000, PAGE_READWRITE, SEC_COMMIT, 0).0, STATUS_OBJECT_NAME_COLLISION);
    assert_eq!(f.mem.live_sections(), backings);
}

#[test]
fn protections_and_parameters() {
    let mut f = Fake::new();
    let (_, ro) = create(&mut f, "", 0x1000, PAGE_READONLY, SEC_COMMIT, 0);
    assert_eq!(map(&mut f, ro, 0, None, 0, PAGE_READWRITE).0, STATUS_SECTION_PROTECTION);
    assert_eq!(map(&mut f, ro, 0, None, 0, PAGE_EXECUTE_READ).0, STATUS_SECTION_PROTECTION);
    let (s, v, _, _) = map(&mut f, ro, 0, None, 0, PAGE_READONLY);
    assert_eq!(s, STATUS_SUCCESS);
    // NtProtectVirtualMemory in a view follows the section too.
    let b = f.put_u64(v);
    let n = f.put_u64(0x1000);
    let old = f.alloc(4);
    let st = f.call(Service::ProtectVirtualMemory, &[SELF, b, n, u64::from(PAGE_READWRITE), old]);
    assert_eq!(st, STATUS_SECTION_PROTECTION);
    let st = f.call(Service::ProtectVirtualMemory, &[SELF, b, n, u64::from(PAGE_NOACCESS), old]);
    assert_eq!((st, f.u32_at(old)), (STATUS_SUCCESS, PAGE_READONLY));
    assert_eq!(create(&mut f, "", 0, PAGE_READWRITE, SEC_COMMIT, 0).0, STATUS_INVALID_PARAMETER_4);
    assert_eq!(create(&mut f, "", 0x1000, PAGE_READWRITE, 0, 0).0, STATUS_INVALID_PARAMETER_6);
    assert_eq!(create(&mut f, "", 0x1000, PAGE_READWRITE, SEC_COMMIT | SEC_RESERVE, 0).0, STATUS_INVALID_PARAMETER_6);
    assert_eq!(create(&mut f, "", 0x1000, PAGE_READWRITE, SEC_IMAGE, 0).0, STATUS_INVALID_FILE_FOR_SECTION);
    assert_eq!(
        create(&mut f, "", 0x1000, PAGE_READWRITE | PAGE_GUARD, SEC_COMMIT, 0).0,
        STATUS_INVALID_PAGE_PROTECTION
    );
    assert_eq!(create(&mut f, "", 1 << 41, PAGE_READWRITE, SEC_COMMIT, 0).0, STATUS_SECTION_TOO_BIG);
    let (_, h) = create(&mut f, "", 0x1000, PAGE_READWRITE, SEC_COMMIT, 0);
    let b = f.put_u64(0);
    let s = f.put_u64(0);
    let bad_inherit = f.call(Service::MapViewOfSection, &[h, SELF, b, 0, 0, 0, s, 3, 0, u64::from(PAGE_READWRITE)]);
    assert_eq!(bad_inherit, STATUS_INVALID_PARAMETER_8);
    // A handle of another type.
    let (_, h_ev) = {
        let out = f.alloc(8);
        let st = f.call(Service::CreateEvent, &[out, u64::from(EVENT_ALL_ACCESS), 0, 0, 0]);
        (st, f.u64_at(out))
    };
    assert_eq!(map(&mut f, h_ev, 0, None, 0, PAGE_READWRITE).0, STATUS_OBJECT_TYPE_MISMATCH);
}

#[test]
fn reserve_sections_commit_through_views() {
    let mut f = Fake::new();
    let (s, h) = create(&mut f, "", 0x4000, PAGE_READWRITE, SEC_RESERVE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (_, v1, _, _) = map_with(&mut f, h, 0, 0x1000, None, 0, PAGE_READWRITE);
    let (_, v2, _, _) = map(&mut f, h, 0, None, 0, PAGE_READWRITE);
    assert_eq!(query(&mut f, v1).region_size, 0x1000);
    assert_eq!(query(&mut f, v1 + 0x1000).state, MEM_RESERVE);
    assert!(f.mem.read(v1 + 0x2000, &mut [0u8; 1]).is_err());
    assert_eq!(allocate(&mut f, v1 + 0x2000, 0x1000, MEM_COMMIT, PAGE_READWRITE).0, STATUS_SUCCESS);
    f.mem.write_bytes(v1 + 0x2000, b"late").unwrap();
    assert_eq!(f.mem.bytes_at(v2 + 0x2000, 4), b"late");
    assert_eq!(query(&mut f, v2 + 0x2000).state, MEM_COMMIT);
    // A view made later sees what was committed.
    let (_, v3, _, _) = map(&mut f, h, 0, None, 0, PAGE_READWRITE);
    assert_eq!(query(&mut f, v3 + 0x2000).state, MEM_COMMIT);
    assert_eq!(query(&mut f, v3 + 0x3000).state, MEM_RESERVE);
}

/// A File object for `bytes`, and a handle to it.
fn file(f: &mut Fake, key: u64, bytes: Vec<u8>) -> u64 {
    f.mem.files.insert(key, bytes);
    let caller = f.caller();
    let new = NewObject::Opaque { kind: ObjectType::File, cookie: key, signaled: None, parse: false };
    let id = f.sys.objects.new_object(new, TID).unwrap();
    let h = f.sys.objects.open_by_pointer(&caller, id, FILE_ALL_ACCESS, 0).unwrap();
    f.sys.objects.dereference(id).unwrap();
    h.0
}

#[test]
fn image_sections_from_files() {
    let mut f = Fake::new();
    let fh = file(&mut f, 3, test_exe(false));
    let (s, h) = create(&mut f, "", 0, PAGE_EXECUTE_WRITECOPY, SEC_IMAGE, fh);
    assert_eq!(s, STATUS_SUCCESS);
    // The executable already sits at the preferred base: the view moves and
    // the caller is told to relocate.
    let (s, v, n, _) = map(&mut f, h, 0, None, 0, PAGE_READONLY);
    assert_eq!((s, n), (STATUS_IMAGE_NOT_AT_BASE, 0x5000));
    assert_ne!(v, IMAGE_BASE);
    let q = query(&mut f, v + 0x1000);
    assert_eq!((q.type_, q.protect, q.allocation_base), (MEM_IMAGE, PAGE_EXECUTE_READ, v));
    // Not relocated: the pointer still names the preferred base.
    assert_eq!(f.u64_at(v + 0x2008), IMAGE_BASE + 0x1000);
    // The section holds the file; both go with the view.
    assert_eq!(f.call(Service::Close, &[h]), STATUS_SUCCESS);
    assert_eq!(f.call(Service::Close, &[fh]), STATUS_SUCCESS);
    assert!(f.mem.deleted_files.is_empty());
    assert_eq!(f.call(Service::UnmapViewOfSection, &[SELF, v]), STATUS_SUCCESS);
    assert_eq!(f.mem.deleted_files, [3]);
    // Not an image.
    let bad = file(&mut f, 4, vec![1, 2, 3]);
    assert_eq!(create(&mut f, "", 0, PAGE_READONLY, SEC_IMAGE, bad).0, STATUS_INVALID_IMAGE_FORMAT);
}

#[test]
fn data_file_sections() {
    let mut f = Fake::new();
    let fh = file(&mut f, 5, b"file contents".to_vec());
    let (s, h) = create(&mut f, "", 0, PAGE_READONLY, SEC_COMMIT, fh);
    assert_eq!(s, STATUS_SUCCESS);
    let (s, v, n, _) = map(&mut f, h, 0, None, 0, PAGE_READONLY);
    assert_eq!((s, n), (STATUS_SUCCESS, 0x1000));
    assert_eq!(f.mem.bytes_at(v, 13), b"file contents");
    assert_eq!(create(&mut f, "", 0x2000, PAGE_READONLY, SEC_COMMIT, fh).0, STATUS_SECTION_TOO_BIG);
    let empty = file(&mut f, 6, Vec::new());
    assert_eq!(create(&mut f, "", 0, PAGE_READONLY, SEC_COMMIT, empty).0, STATUS_MAPPED_FILE_SIZE_ZERO);
}
