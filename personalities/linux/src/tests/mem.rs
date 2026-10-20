//! mmap, munmap, mprotect, brk.

use super::fake::*;
use crate::abi::{PAGE_SIZE, map, prot};
use crate::kernel::{Backing, MapHints, MapRequest, Placement, Prot};
use crate::{Errno, nr};

const RW_PROT: u64 = (prot::PROT_READ | prot::PROT_WRITE) as u64;
const PRIV_ANON: u64 = (map::MAP_PRIVATE | map::MAP_ANONYMOUS) as u64;

fn mmap(k: &mut FakeKernel, addr: u64, len: u64, prot: u64, flags: u64, fd: u64, off: u64) -> Result<u64, Errno> {
    sys(k, nr::MMAP, &[addr, len, prot, flags, fd, off])
}

#[test]
fn anonymous_private_mapping() {
    let mut k = FakeKernel::new();
    let flags = PRIV_ANON | map::MAP_STACK as u64;
    let addr = mmap(&mut k, 0, 16048, RW_PROT, flags, u64::MAX, 0).unwrap();
    assert_eq!(addr % PAGE_SIZE, 0);
    assert_eq!(
        k.maps[0],
        MapRequest {
            addr: 0,
            len: 16384,
            prot: Prot(RW_PROT as u32),
            placement: Placement::Hint,
            shared: false,
            backing: Backing::Anonymous,
            hints: MapHints { stack: true, ..Default::default() },
        }
    );
    assert!(k.is_mapped(addr) && k.is_mapped(addr + 16383) && !k.is_mapped(addr + 16384));
    assert_eq!(k.peek(addr, 16), [0; 16], "anonymous memory is zeroed");
}

#[test]
fn mapping_types() {
    let mut k = FakeKernel::new();
    let anon = map::MAP_ANONYMOUS as u64;
    assert_eq!(mmap(&mut k, 0, 4096, RW_PROT, anon, u64::MAX, 0), Err(Errno::EINVAL), "no type");
    assert_eq!(mmap(&mut k, 0, 4096, RW_PROT, anon | 0x4, u64::MAX, 0), Err(Errno::EINVAL), "type 4");
    assert!(mmap(&mut k, 0, 4096, RW_PROT, anon | map::MAP_SHARED as u64, u64::MAX, 0).is_ok());
    assert!(k.maps[0].shared);
    assert!(mmap(&mut k, 0, 4096, RW_PROT, anon | map::MAP_SHARED_VALIDATE as u64, u64::MAX, 0).is_ok());
    assert!(k.maps[1].shared);
}

#[test]
fn shared_validate_rejects_unknown_flags_others_ignore_them() {
    let mut k = FakeKernel::new();
    let unknown = 0x200u64; // not defined by mmap(2)
    let validate = (map::MAP_SHARED_VALIDATE | map::MAP_ANONYMOUS) as u64;
    assert_eq!(mmap(&mut k, 0, 4096, RW_PROT, validate | unknown, u64::MAX, 0), Err(Errno::EOPNOTSUPP));
    assert_eq!(
        mmap(&mut k, 0, 4096, RW_PROT, validate | map::MAP_SYNC as u64, u64::MAX, 0),
        Err(Errno::EOPNOTSUPP),
        "MAP_SYNC needs a DAX file"
    );
    assert!(mmap(&mut k, 0, 4096, RW_PROT, PRIV_ANON | unknown, u64::MAX, 0).is_ok());
    let shared = (map::MAP_SHARED | map::MAP_ANONYMOUS | map::MAP_SYNC) as u64;
    assert!(mmap(&mut k, 0, 4096, RW_PROT, shared | unknown, u64::MAX, 0).is_ok(), "MAP_SHARED ignores MAP_SYNC");
    let known = (map::MAP_DENYWRITE | map::MAP_EXECUTABLE | map::MAP_NONBLOCK | map::MAP_UNINITIALIZED) as u64;
    assert!(mmap(&mut k, 0, 4096, RW_PROT, validate | known, u64::MAX, 0).is_ok());
}

#[test]
fn length_and_alignment_checks() {
    let mut k = FakeKernel::new();
    assert_eq!(mmap(&mut k, 0, 0, RW_PROT, PRIV_ANON, u64::MAX, 0), Err(Errno::EINVAL), "length 0");
    assert_eq!(mmap(&mut k, 0, 4096, RW_PROT, PRIV_ANON, u64::MAX, 100), Err(Errno::EINVAL), "offset");
    let fixed = PRIV_ANON | map::MAP_FIXED as u64;
    assert_eq!(mmap(&mut k, 0x100_0001, 4096, RW_PROT, fixed, u64::MAX, 0), Err(Errno::EINVAL), "fixed addr");
    let noreplace = PRIV_ANON | map::MAP_FIXED_NOREPLACE as u64;
    assert_eq!(mmap(&mut k, 0x100_0800, 4096, RW_PROT, noreplace, u64::MAX, 0), Err(Errno::EINVAL));
    assert_eq!(
        mmap(&mut k, 0, u64::MAX - 10, RW_PROT, PRIV_ANON, u64::MAX, 0),
        Err(Errno::ENOMEM),
        "rounding overflows"
    );
    assert!(k.maps.is_empty(), "nothing reached the kernel");
}

#[test]
fn an_unaligned_hint_is_only_a_hint() {
    let mut k = FakeKernel::new();
    let addr = mmap(&mut k, 0x200_0123, 1, RW_PROT, PRIV_ANON, u64::MAX, 0).unwrap();
    assert_eq!(addr, 0x200_0000);
    assert_eq!(k.maps[0].len, PAGE_SIZE);
}

#[test]
fn prot_checks() {
    let mut k = FakeKernel::new();
    assert_eq!(mmap(&mut k, 0, 4096, 0x10, PRIV_ANON, u64::MAX, 0), Err(Errno::EINVAL));
    let grows = prot::PROT_GROWSDOWN as u64;
    assert_eq!(mmap(&mut k, 0, 4096, grows, PRIV_ANON, u64::MAX, 0), Err(Errno::EINVAL), "mprotect-only bit");
    let all = (prot::PROT_READ | prot::PROT_WRITE | prot::PROT_EXEC | prot::PROT_SEM) as u64;
    assert!(mmap(&mut k, 0, 4096, all, PRIV_ANON, u64::MAX, 0).is_ok());
    assert!(mmap(&mut k, 0, 4096, prot::PROT_NONE as u64, PRIV_ANON, u64::MAX, 0).is_ok());
}

#[test]
fn fixed_replaces_and_noreplace_refuses() {
    let mut k = FakeKernel::new();
    k.poke(DATA, b"old");
    let fixed = PRIV_ANON | map::MAP_FIXED as u64;
    assert_eq!(mmap(&mut k, DATA, 4096, RW_PROT, fixed, u64::MAX, 0), Ok(DATA));
    assert_eq!(k.peek(DATA, 3), [0, 0, 0], "the old page was replaced");
    let noreplace = PRIV_ANON | map::MAP_FIXED_NOREPLACE as u64;
    assert_eq!(mmap(&mut k, DATA, 4096, RW_PROT, noreplace, u64::MAX, 0), Err(Errno::EEXIST));
    assert_eq!(mmap(&mut k, 0x900_0000, 4096, RW_PROT, noreplace, u64::MAX, 0), Ok(0x900_0000));
    // Both flags: the stricter NOREPLACE wins.
    let both = fixed | map::MAP_FIXED_NOREPLACE as u64;
    assert_eq!(mmap(&mut k, DATA, 4096, RW_PROT, both, u64::MAX, 0), Err(Errno::EEXIST));
    // Fixed beyond user space or at the bottom: the kernel refuses.
    assert_eq!(mmap(&mut k, USER_END, 4096, RW_PROT, fixed, u64::MAX, 0), Err(Errno::ENOMEM));
    assert_eq!(mmap(&mut k, 0, 4096, RW_PROT, fixed, u64::MAX, 0), Err(Errno::EPERM));
}

#[test]
fn map_32bit_is_a_hint_and_ignored_when_fixed() {
    let mut k = FakeKernel::new();
    let low = PRIV_ANON | map::MAP_32BIT as u64;
    mmap(&mut k, 0, 4096, RW_PROT, low, u64::MAX, 0).unwrap();
    assert!(k.maps[0].hints.low_2g);
    mmap(&mut k, 0x900_0000, 4096, RW_PROT, low | map::MAP_FIXED as u64, u64::MAX, 0).unwrap();
    assert!(!k.maps[1].hints.low_2g);
    let all_hints = PRIV_ANON
        | (map::MAP_GROWSDOWN | map::MAP_POPULATE | map::MAP_LOCKED | map::MAP_NORESERVE | map::MAP_STACK) as u64;
    mmap(&mut k, 0, 4096, RW_PROT, all_hints, u64::MAX, 0).unwrap();
    assert_eq!(
        k.maps[2].hints,
        MapHints { low_2g: false, grows_down: true, stack: true, populate: true, locked: true, no_reserve: true }
    );
}

#[test]
fn hugetlb_has_no_pool() {
    let mut k = FakeKernel::new();
    let huge = PRIV_ANON | map::MAP_HUGETLB as u64 | (21 << map::MAP_HUGE_SHIFT);
    assert_eq!(mmap(&mut k, 0, 2 << 20, RW_PROT, huge, u64::MAX, 0), Err(Errno::ENOMEM));
}

#[test]
fn file_mappings() {
    let mut k = FakeKernel::new();
    let private = map::MAP_PRIVATE as u64;
    assert_eq!(mmap(&mut k, 0, 4096, RW_PROT, private, u64::MAX, 0), Err(Errno::EBADF), "fd -1");
    assert_eq!(mmap(&mut k, 0, 4096, RW_PROT, private, 9, 0), Err(Errno::EBADF), "closed fd");
    assert_eq!(mmap(&mut k, 0, 4096, RW_PROT, private, 1, 0), Err(Errno::ENODEV), "a pipe");
    assert_eq!(
        mmap(&mut k, 0, 8192, RW_PROT, private, 3, u64::MAX - 4095),
        Err(Errno::EINVAL),
        "offset + length overflows"
    );
    let mut f = File::new(Kind::Regular);
    f.written = (0..8192u32).map(|i| (i / 4096) as u8 + 1).collect();
    k.files.insert(3, f);
    let addr = mmap(&mut k, 0, 4096, prot::PROT_READ as u64, private, 3, 4096).unwrap();
    assert_eq!(k.maps.last().unwrap().backing, Backing::File { fd: 3, offset: 4096 });
    assert_eq!(k.peek(addr, 2), [2, 2]);
    // With MAP_ANONYMOUS the fd is ignored.
    assert!(mmap(&mut k, 0, 4096, RW_PROT, PRIV_ANON, 12345, 0).is_ok());
}

#[test]
fn munmap_rounds_and_validates() {
    let mut k = FakeKernel::new();
    let addr = mmap(&mut k, 0, 3 * PAGE_SIZE, RW_PROT, PRIV_ANON, u64::MAX, 0).unwrap();
    assert_eq!(sys(&mut k, nr::MUNMAP, &[addr + PAGE_SIZE, 1]), Ok(0));
    assert!(k.is_mapped(addr) && !k.is_mapped(addr + PAGE_SIZE) && k.is_mapped(addr + 2 * PAGE_SIZE));
    assert_eq!(sys(&mut k, nr::MUNMAP, &[addr + PAGE_SIZE, PAGE_SIZE]), Ok(0), "holes are fine");
    assert_eq!(sys(&mut k, nr::MUNMAP, &[addr + 1, PAGE_SIZE]), Err(Errno::EINVAL), "unaligned");
    assert_eq!(sys(&mut k, nr::MUNMAP, &[addr, 0]), Err(Errno::EINVAL), "length 0");
    assert_eq!(sys(&mut k, nr::MUNMAP, &[addr, u64::MAX]), Err(Errno::EINVAL), "rounding overflows");
    assert_eq!(sys(&mut k, nr::MUNMAP, &[u64::MAX - 4095, 8192]), Err(Errno::EINVAL), "wraps");
    assert_eq!(sys(&mut k, nr::MUNMAP, &[USER_END, 4096]), Err(Errno::EINVAL), "beyond user space");
}

#[test]
fn mprotect_rounds_and_validates() {
    let mut k = FakeKernel::new();
    let addr = mmap(&mut k, 0, 4 * PAGE_SIZE, RW_PROT, PRIV_ANON, u64::MAX, 0).unwrap();
    assert_eq!(sys(&mut k, nr::MPROTECT, &[addr, 4096, prot::PROT_NONE as u64]), Ok(0));
    assert_eq!(k.prot_at(addr), Some(0));
    assert_eq!(sys(&mut k, nr::MPROTECT, &[addr + PAGE_SIZE, 1, prot::PROT_READ as u64]), Ok(0));
    assert_eq!(k.protects.last().unwrap().1, PAGE_SIZE, "length rounded up");
    assert_eq!(k.prot_at(addr + PAGE_SIZE), Some(prot::PROT_READ));
    assert_eq!(sys(&mut k, nr::MPROTECT, &[addr + 1, 4096, 0]), Err(Errno::EINVAL), "unaligned");
    assert_eq!(sys(&mut k, nr::MPROTECT, &[addr, 4096, 0x40]), Err(Errno::EINVAL), "unknown bit");
    let both = (prot::PROT_GROWSDOWN | prot::PROT_GROWSUP) as u64;
    assert_eq!(sys(&mut k, nr::MPROTECT, &[addr, 4096, both]), Err(Errno::EINVAL));
    let down = (prot::PROT_READ | prot::PROT_GROWSDOWN) as u64;
    assert_eq!(sys(&mut k, nr::MPROTECT, &[addr, 4096, down]), Ok(0));
    assert!(k.protects.last().unwrap().2.grows_down());
    assert_eq!(sys(&mut k, nr::MPROTECT, &[UNMAPPED, 4096, 0]), Err(Errno::ENOMEM), "not mapped");
    assert_eq!(sys(&mut k, nr::MPROTECT, &[addr, 5 * PAGE_SIZE, 0]), Err(Errno::ENOMEM), "runs past the mapping");
    assert_eq!(sys(&mut k, nr::MPROTECT, &[u64::MAX - 4095, 8192, 0]), Err(Errno::ENOMEM), "wraps");
    let before = k.protects.len();
    assert_eq!(sys(&mut k, nr::MPROTECT, &[UNMAPPED, 0, 0]), Ok(0), "size 0 covers no page");
    assert_eq!(k.protects.len(), before);
}

#[test]
fn brk_returns_the_break_and_never_an_error() {
    let mut k = FakeKernel::new();
    let start = k.cur_brk;
    assert_eq!(sys(&mut k, nr::BRK, &[0]), Ok(start), "query");
    assert_eq!(sys(&mut k, nr::BRK, &[start + 0xd00]), Ok(start + 0xd00), "not rounded");
    assert!(k.is_mapped(start) && !k.is_mapped(start + PAGE_SIZE));
    assert_eq!(sys(&mut k, nr::BRK, &[start + 0x21d00]), Ok(start + 0x21d00));
    assert!(k.is_mapped(start + 0x21000));
    // Below the start, or into another mapping: refused, current break returned.
    assert_eq!(sys(&mut k, nr::BRK, &[start - 1]), Ok(start + 0x21d00));
    k.map_range(start + 0x40000, PAGE_SIZE, RW);
    assert_eq!(sys(&mut k, nr::BRK, &[start + 0x50000]), Ok(start + 0x21d00));
    assert_eq!(sys(&mut k, nr::BRK, &[u64::MAX]), Ok(start + 0x21d00));
    // Shrinking works and unmaps.
    assert_eq!(sys(&mut k, nr::BRK, &[start + 0x10]), Ok(start + 0x10));
    assert!(!k.is_mapped(start + PAGE_SIZE));
}
