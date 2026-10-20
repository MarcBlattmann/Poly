//! madvise(2) (ADR 0049).

use super::fake::*;
use crate::abi::{PAGE_SIZE, map, prot};
use crate::sys::madvise::*;
use crate::{Errno, nr};

const RW_PROT: u64 = (prot::PROT_READ | prot::PROT_WRITE) as u64;
const PRIV_ANON: u64 = (map::MAP_PRIVATE | map::MAP_ANONYMOUS) as u64;

fn madvise(k: &mut FakeKernel, addr: u64, len: u64, advice: u32) -> Result<u64, Errno> {
    sys(k, nr::MADVISE, &[addr, len, u64::from(advice)])
}

#[test]
fn advice_values_sort_into_what_the_kernel_does() {
    assert_eq!(Advice::from_raw(MADV_DONTNEED), Some(Advice::DontNeed));
    assert_eq!(Advice::from_raw(MADV_DONTNEED_LOCKED), Some(Advice::DontNeed));
    assert_eq!(Advice::from_raw(MADV_FREE), Some(Advice::Free));
    assert_eq!(Advice::from_raw(MADV_REMOVE), Some(Advice::Remove));
    let hints = [
        MADV_NORMAL,
        MADV_RANDOM,
        MADV_SEQUENTIAL,
        MADV_WILLNEED,
        MADV_DONTFORK,
        MADV_DOFORK,
        MADV_MERGEABLE,
        MADV_UNMERGEABLE,
        MADV_HUGEPAGE,
        MADV_NOHUGEPAGE,
        MADV_DONTDUMP,
        MADV_DODUMP,
        MADV_COLD,
        MADV_PAGEOUT,
    ];
    for h in hints {
        assert_eq!(Advice::from_raw(h), Some(Advice::Hint), "{h}");
    }
    // Refused like a kernel that predates them; unknown values.
    let refused = [
        5,
        6,
        7,
        MADV_WIPEONFORK,
        MADV_KEEPONFORK,
        MADV_POPULATE_READ,
        MADV_POPULATE_WRITE,
        MADV_COLLAPSE,
        26,
        MADV_HWPOISON,
        MADV_SOFT_OFFLINE,
        102,
        103,
        u32::MAX,
    ];
    for r in refused {
        assert_eq!(Advice::from_raw(r), None, "{r}");
    }
}

#[test]
fn dontneed_reaches_the_kernel_with_whole_pages() {
    let mut k = FakeKernel::new();
    let addr = sys(&mut k, nr::MMAP, &[0, 3 * PAGE_SIZE, RW_PROT, PRIV_ANON, u64::MAX, 0]).unwrap();
    k.poke(addr + PAGE_SIZE, b"stack of a finished thread");
    assert_eq!(madvise(&mut k, addr, 2 * PAGE_SIZE - 100, MADV_DONTNEED), Ok(0));
    assert_eq!(k.advised, [(addr, 2 * PAGE_SIZE, Advice::DontNeed)], "the length is rounded up to pages");
    assert_eq!(k.peek(addr + PAGE_SIZE, 5), [0; 5], "private anonymous memory reads as zeros again");
    assert_eq!(madvise(&mut k, addr, PAGE_SIZE, MADV_WILLNEED), Ok(0));
    assert_eq!(k.advised.last(), Some(&(addr, PAGE_SIZE, Advice::Hint)));
    // The kernel's ENOMEM for a range that is not all mapped comes back.
    assert_eq!(madvise(&mut k, addr, 4 * PAGE_SIZE, MADV_DONTNEED), Err(Errno::ENOMEM));
    assert_eq!(madvise(&mut k, UNMAPPED, PAGE_SIZE, MADV_NORMAL), Err(Errno::ENOMEM));
}

#[test]
fn bad_arguments_never_reach_the_kernel() {
    let mut k = FakeKernel::new();
    let addr = sys(&mut k, nr::MMAP, &[0, PAGE_SIZE, RW_PROT, PRIV_ANON, u64::MAX, 0]).unwrap();
    assert_eq!(madvise(&mut k, addr, PAGE_SIZE, 7), Err(Errno::EINVAL), "unknown advice");
    assert_eq!(madvise(&mut k, addr, PAGE_SIZE, MADV_WIPEONFORK), Err(Errno::EINVAL), "not supported");
    assert_eq!(madvise(&mut k, addr + 1, PAGE_SIZE, MADV_DONTNEED), Err(Errno::EINVAL), "unaligned");
    assert_eq!(madvise(&mut k, addr, u64::MAX, MADV_DONTNEED), Err(Errno::EINVAL), "negative length");
    assert_eq!(madvise(&mut k, u64::MAX - 4095, 8192, MADV_DONTNEED), Err(Errno::EINVAL), "wraps");
    // The advice is checked before the address.
    assert_eq!(madvise(&mut k, addr + 1, PAGE_SIZE, 7), Err(Errno::EINVAL));
    assert_eq!(madvise(&mut k, addr, 0, MADV_DONTNEED), Ok(0), "length 0 succeeds");
    assert_eq!(madvise(&mut k, UNMAPPED, 0, MADV_DONTNEED), Ok(0), "even where nothing is mapped");
    assert!(k.advised.is_empty());
}
