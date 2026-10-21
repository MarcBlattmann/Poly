use super::*;
use crate::consts::*;
use crate::vm::MemoryBasicInformation;
use poly_ntobj::status::*;

/// `NtAllocateVirtualMemory`: (status, base, size).
pub fn allocate(f: &mut Fake, base: u64, size: u64, kind: u32, protect: u32) -> (NtStatus, u64, u64) {
    allocate_bits(f, base, 0, size, kind, protect)
}

fn allocate_bits(f: &mut Fake, base: u64, zero_bits: u64, size: u64, kind: u32, protect: u32) -> (NtStatus, u64, u64) {
    let b = f.put_u64(base);
    let s = f.put_u64(size);
    let st = f.call(Service::AllocateVirtualMemory, &[SELF, b, zero_bits, s, u64::from(kind), u64::from(protect)]);
    (st, f.u64_at(b), f.u64_at(s))
}

fn free(f: &mut Fake, base: u64, size: u64, kind: u32) -> (NtStatus, u64, u64) {
    let b = f.put_u64(base);
    let s = f.put_u64(size);
    let st = f.call(Service::FreeVirtualMemory, &[SELF, b, s, u64::from(kind)]);
    (st, f.u64_at(b), f.u64_at(s))
}

fn protect(f: &mut Fake, base: u64, size: u64, prot: u32) -> (NtStatus, u32) {
    let b = f.put_u64(base);
    let s = f.put_u64(size);
    let old = f.alloc(4);
    let st = f.call(Service::ProtectVirtualMemory, &[SELF, b, s, u64::from(prot), old]);
    (st, f.u32_at(old))
}

pub fn query(f: &mut Fake, addr: u64) -> MemoryBasicInformation {
    let buf = f.alloc(48);
    let ret = f.alloc(8);
    assert_eq!(f.call(Service::QueryVirtualMemory, &[SELF, addr, 0, buf, 48, ret]), STATUS_SUCCESS);
    assert_eq!(f.u64_at(ret), 48);
    MemoryBasicInformation {
        base_address: f.u64_at(buf),
        allocation_base: f.u64_at(buf + 8),
        allocation_protect: f.u32_at(buf + 16),
        region_size: f.u64_at(buf + 24),
        state: f.u32_at(buf + 32),
        protect: f.u32_at(buf + 36),
        type_: f.u32_at(buf + 40),
    }
}

#[test]
fn reserve_commit_protect_decommit_release() {
    let mut f = Fake::new();
    let (s, base, size) = allocate(&mut f, 0, 0x3000, MEM_RESERVE, PAGE_READWRITE);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!((base % 0x1_0000, size), (0, 0x3000));
    let q = query(&mut f, base + 0x10);
    assert_eq!((q.base_address, q.allocation_base, q.region_size), (base, base, 0x3000));
    assert_eq!((q.state, q.protect, q.type_, q.allocation_protect), (MEM_RESERVE, 0, MEM_PRIVATE, PAGE_READWRITE));
    // Reserved memory cannot be touched.
    assert!(f.mem.read(base, &mut [0u8; 1]).is_err());

    // Commit the middle page.
    let (s, b, n) = allocate(&mut f, base + 0x1800, 0x10, MEM_COMMIT, PAGE_READWRITE);
    assert_eq!((s, b, n), (STATUS_SUCCESS, base + 0x1000, 0x1000));
    f.mem.write_bytes(base + 0x1000, b"poly").unwrap();
    let q = query(&mut f, base + 0x1000);
    assert_eq!((q.state, q.protect, q.region_size), (MEM_COMMIT, PAGE_READWRITE, 0x1000));
    assert_eq!(query(&mut f, base).region_size, 0x1000);
    // Committing again keeps the contents.
    assert_eq!(allocate(&mut f, base + 0x1000, 0x1000, MEM_COMMIT, PAGE_READWRITE).0, STATUS_SUCCESS);
    assert_eq!(f.mem.bytes_at(base + 0x1000, 4), b"poly");

    // Read-only: reads work, writes fault.
    assert_eq!(protect(&mut f, base + 0x1000, 0x1000, PAGE_READONLY), (STATUS_SUCCESS, PAGE_READWRITE));
    assert_eq!(f.mem.bytes_at(base + 0x1000, 4), b"poly");
    assert!(f.mem.write_bytes(base + 0x1000, b"x").is_err());
    assert_eq!(query(&mut f, base + 0x1000).protect, PAGE_READONLY);
    // A guard page.
    assert_eq!(protect(&mut f, base + 0x1000, 1, PAGE_READWRITE | PAGE_GUARD), (STATUS_SUCCESS, PAGE_READONLY));
    assert_eq!(query(&mut f, base + 0x1000).protect, PAGE_READWRITE | PAGE_GUARD);
    assert!(f.mem.read(base + 0x1000, &mut [0u8; 1]).is_err());
    // Its first access fires once.
    let fault = crate::exec::access_fault(&mut f.proc, &mut f.mem, TID, base + 0x1000);
    assert_eq!(fault, crate::vm::AccessFault::Exception(STATUS_GUARD_PAGE_VIOLATION));
    assert_eq!(query(&mut f, base + 0x1000).protect, PAGE_READWRITE);
    assert_eq!(f.mem.bytes_at(base + 0x1000, 4), b"poly");
    // Uncommitted pages cannot be protected.
    assert_eq!(protect(&mut f, base, 0x2000, PAGE_READONLY).0, STATUS_NOT_COMMITTED);

    // Decommit: the contents go.
    assert_eq!(free(&mut f, base + 0x1000, 0x1000, MEM_DECOMMIT), (STATUS_SUCCESS, base + 0x1000, 0x1000));
    assert_eq!(query(&mut f, base + 0x1000).state, MEM_RESERVE);
    allocate(&mut f, base + 0x1000, 0x1000, MEM_COMMIT, PAGE_READWRITE);
    assert_eq!(f.mem.bytes_at(base + 0x1000, 4), [0; 4]);

    // Release needs the base and a size of 0.
    assert_eq!(free(&mut f, base + 0x1000, 0, MEM_RELEASE).0, STATUS_FREE_VM_NOT_AT_BASE);
    assert_eq!(free(&mut f, base, 0x1000, MEM_RELEASE).0, STATUS_UNABLE_TO_FREE_VM);
    assert_eq!(free(&mut f, base, 0, MEM_RELEASE), (STATUS_SUCCESS, base, 0x3000));
    let q = query(&mut f, base);
    assert_eq!((q.state, q.protect, q.allocation_base), (MEM_FREE, PAGE_NOACCESS, 0));
    assert_eq!(free(&mut f, base, 0, MEM_RELEASE).0, STATUS_MEMORY_NOT_ALLOCATED);
    assert!(f.mem.rights(base + 0x1000).is_none());
}

#[test]
fn placement() {
    let mut f = Fake::new();
    // A fixed base is rounded down to 64 KiB, the end up to a page.
    let (s, b, n) = allocate(&mut f, 0x5000_1234, 0x10, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
    assert_eq!((s, b, n), (STATUS_SUCCESS, 0x5000_0000, 0x2000));
    assert_eq!(allocate(&mut f, 0x5000_0000, 0x1000, MEM_RESERVE, PAGE_READWRITE).0, STATUS_CONFLICTING_ADDRESSES);
    // Bottom up by default, top down on request.
    let (_, low, _) = allocate(&mut f, 0, 0x1000, MEM_RESERVE, PAGE_READWRITE);
    let (_, high, _) = allocate(&mut f, 0, 0x1000, MEM_RESERVE | MEM_TOP_DOWN, PAGE_READWRITE);
    assert!(low < 0x5000_0000, "{low:#x}");
    assert!(high > 0x7F00_0000_0000, "{high:#x}");
    assert_eq!(high % 0x1_0000, 0);
    // Zero bits limit the base.
    let (s, b, _) = allocate_bits(&mut f, 0, 20, 0x1000, MEM_RESERVE | MEM_TOP_DOWN, PAGE_READWRITE);
    assert_eq!(s, STATUS_SUCCESS);
    assert!(((1 << 44) - 0x10_0000..1 << 44).contains(&b), "{b:#x}");
    assert_eq!(allocate_bits(&mut f, 0, 21, 0x1000, MEM_RESERVE, PAGE_READWRITE).0, STATUS_INVALID_PARAMETER_3);
    // Commit without a base reserves too.
    let (s, b, _) = allocate(&mut f, 0, 0x1000, MEM_COMMIT, PAGE_READWRITE);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(query(&mut f, b).state, MEM_COMMIT);
    // Committing outside a reservation.
    assert_eq!(allocate(&mut f, 0x6000_0000, 0x1000, MEM_COMMIT, PAGE_READWRITE).0, STATUS_CONFLICTING_ADDRESSES);
    // Out of user space.
    assert_eq!(allocate(&mut f, 0x1000, 0x1000, MEM_RESERVE, PAGE_READWRITE).0, STATUS_INVALID_PARAMETER_2);
    // Free space is reported up to the next allocation.
    let q = query(&mut f, 0x5000_0000 - 0x1000);
    assert_eq!((q.state, q.base_address + q.region_size), (MEM_FREE, 0x5000_0000));
}

#[test]
fn argument_errors() {
    let mut f = Fake::new();
    let rw = PAGE_READWRITE;
    assert_eq!(allocate(&mut f, 0, 0x1000, MEM_RESERVE | 0x1, rw).0, STATUS_INVALID_PARAMETER_5);
    assert_eq!(allocate(&mut f, 0, 0x1000, MEM_TOP_DOWN, rw).0, STATUS_INVALID_PARAMETER_5);
    assert_eq!(allocate(&mut f, 0, 0, MEM_RESERVE, rw).0, STATUS_INVALID_PARAMETER_4);
    assert_eq!(allocate(&mut f, 0, 0x1000, MEM_RESERVE, PAGE_WRITECOPY).0, STATUS_INVALID_PAGE_PROTECTION);
    assert_eq!(allocate(&mut f, 0, 0x1000, MEM_RESERVE, 0).0, STATUS_INVALID_PAGE_PROTECTION);
    assert_eq!(allocate(&mut f, 0, 0x1000, MEM_RESERVE | MEM_LARGE_PAGES, rw).0, STATUS_NOT_SUPPORTED);
    assert_eq!(free(&mut f, 0x5000_0000, 0, MEM_RELEASE | MEM_DECOMMIT).0, STATUS_INVALID_PARAMETER_4);
    let buf = f.alloc(48);
    assert_eq!(f.call(Service::QueryVirtualMemory, &[SELF, 0, 1, buf, 48, 0]), STATUS_INVALID_INFO_CLASS);
    assert_eq!(f.call(Service::QueryVirtualMemory, &[SELF, 0, 0, buf, 40, 0]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(f.call(Service::QueryVirtualMemory, &[SELF, 1 << 47, 0, buf, 48, 0]), STATUS_INVALID_PARAMETER_2);
    assert_eq!(f.call(Service::QueryVirtualMemory, &[0, 0, 0, buf, 48, 0]), STATUS_INVALID_HANDLE);
    // MEM_RESET keeps committed memory.
    let (_, b, _) = allocate(&mut f, 0, 0x1000, MEM_COMMIT, rw);
    assert_eq!(allocate(&mut f, b, 0x1000, MEM_RESET, rw).0, STATUS_SUCCESS);
    assert_eq!(allocate(&mut f, b, 0x1000, MEM_RESET | MEM_COMMIT, rw).0, STATUS_INVALID_PARAMETER_5);
}
