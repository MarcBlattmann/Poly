//! Behaviours of ADR 0064 (spec-a's entries): memory locking and
//! residency, the older time-setting calls, reads and writes at a partly
//! bad buffer, `RLIMIT_FSIZE`, CPU-time clock ids.

extern crate std;

use super::fake::*;
use super::files::{AT, env, s};
use crate::abi::PAGE_SIZE;
use crate::abi::clock as id;
use crate::fs::rlimit::RLIMIT_FSIZE;
use crate::kernel::{CpuTimes, CpuTimesOf, MemLock};
use crate::rlimit::Rlimit;
use crate::time::cpuclock::{Counts, CpuClock, decode};
use crate::{Errno, nr};

const P: u64 = PAGE_SIZE;
/// The first unmapped page after the fake's data area (16 pages).
const DATA_END: u64 = DATA + 16 * P;

// ---- mlock, munlock, mlockall, mincore -------------------------------------

#[test]
fn mlock_rounds_to_pages_and_checks_the_range() {
    let mut k = FakeKernel::new();
    // With CAP_IPC_LOCK: no RLIMIT_MEMLOCK in the way.
    *k.creds_mut() = crate::cred::Creds::root();
    assert_eq!(sys(&mut k, nr::MLOCK, &[DATA + 7, 3]), Ok(0));
    assert_eq!(sys(&mut k, nr::MUNLOCK, &[DATA + P - 1, 2]), Ok(0), "two pages touched");
    assert_eq!(k.locked, [(DATA, P, MemLock::Lock), (DATA, 2 * P, MemLock::Unlock)]);
    assert_eq!(sys(&mut k, nr::MLOCK, &[DATA, 0]), Ok(0), "length 0");
    assert_eq!(sys(&mut k, nr::MLOCK, &[DATA, (-(P as i64)) as u64]), Err(Errno::EINVAL), "wraps");
    assert_eq!(k.locked.len(), 2, "neither reached the kernel");
    assert_eq!(sys(&mut k, nr::MLOCK, &[DATA_END - P, 2 * P]), Err(Errno::ENOMEM), "runs into a hole");
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, P, 1]), Ok(0), "MLOCK_ONFAULT");
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, P, 2]), Err(Errno::EINVAL));
}

#[test]
fn mlockall_flags() {
    let mut k = FakeKernel::new();
    for (flags, want) in [(1, Ok(0)), (2, Ok(0)), (3 | 4, Ok(0)), (0, Err(Errno::EINVAL)), (4, Err(Errno::EINVAL))] {
        assert_eq!(sys(&mut k, nr::MLOCKALL, &[flags]), want, "flags {flags}");
    }
    assert_eq!(sys(&mut k, nr::MLOCKALL, &[8]), Err(Errno::EINVAL), "unknown bit");
    assert_eq!(sys(&mut k, nr::MUNLOCKALL, &[]), Ok(0));
}

#[test]
fn memlock_limit_without_cap_ipc_lock() {
    use crate::cred::Creds;
    use crate::fs::rlimit::RLIMIT_MEMLOCK;
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::user(1000, 1000);
    k.me().limits.set(RLIMIT_MEMLOCK, Rlimit { cur: 2 * P, max: 2 * P });
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, 2 * P, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA + 2 * P, P, 1]), Err(Errno::ENOMEM), "a third page");
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, 2 * P, 1]), Ok(0), "locked pages count once");
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, 3 * P, 0]), Err(Errno::ENOMEM));
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, (-8192i64) as u64, 0]), Err(Errno::ENOMEM), "over any limit");
    assert_eq!(sys(&mut k, nr::MUNLOCK, &[DATA, P]), Ok(0));
    assert_eq!(sys(&mut k, nr::MLOCK, &[DATA + 4 * P, P]), Ok(0));
    assert_eq!(sys(&mut k, nr::MLOCKALL, &[1]), Err(Errno::ENOMEM), "the address space is larger");
    assert_eq!(sys(&mut k, nr::MUNLOCKALL, &[]), Ok(0));
    assert_eq!(sys(&mut k, nr::MLOCKALL, &[2]), Ok(0));
    assert_eq!(crate::sys::mlock::future_mmap_check(&mut k, 3 * P), Err(Errno::EAGAIN));
    assert_eq!(crate::sys::mlock::future_mmap_check(&mut k, P), Ok(Some(MemLock::Lock)));
    k.me().limits.set(RLIMIT_MEMLOCK, Rlimit { cur: 0, max: 2 * P });
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, 0, 0]), Err(Errno::EPERM), "even len 0");
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, P, 2]), Err(Errno::EINVAL), "flags first");
    assert_eq!(sys(&mut k, nr::MLOCKALL, &[2]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::MUNLOCK, &[DATA, P]), Ok(0));
}

#[test]
fn mlock_lengths_round_with_the_offset() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = crate::cred::Creds::root();
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, !0, 0]), Ok(0), "rounds to 0");
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA, (-8192i64) as u64, 0]), Err(Errno::EINVAL), "wraps");
    assert_eq!(sys(&mut k, nr::MLOCK2, &[DATA + P - 1, 2, 1]), Ok(0));
    assert_eq!(k.locked.last(), Some(&(DATA, 2 * P, MemLock::LockOnFault)));
    assert_eq!(sys(&mut k, nr::MLOCK2, &[0xffff_8000_0000_0000, 0, 0]), Ok(0), "len 0 looks at nothing");
}

#[test]
fn mincore_one_byte_per_page() {
    let mut k = FakeKernel::new();
    let vec = scratch(8 * P);
    k.poke(vec, &[0xee; 8]);
    assert_eq!(sys(&mut k, nr::MINCORE, &[DATA, 3 * P + 1, vec]), Ok(0));
    assert_eq!(k.peek(vec, 8), [1, 1, 1, 1, 0xee, 0xee, 0xee, 0xee], "four pages; the rest untouched");
    assert_eq!(sys(&mut k, nr::MINCORE, &[DATA + 1, P, vec]), Err(Errno::EINVAL), "unaligned");
    assert_eq!(sys(&mut k, nr::MINCORE, &[DATA_END - P, 2 * P, vec]), Err(Errno::ENOMEM));
    assert_eq!(sys(&mut k, nr::MINCORE, &[DATA, P, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::MINCORE, &[DATA, 0, UNMAPPED]), Ok(0), "length 0");
}

// ---- utimes, futimesat, utime ------------------------------------------------

/// `(atime, mtime)` of `/dir/file` as `(sec, nsec)` pairs.
fn times_of(k: &mut FakeKernel) -> ((i64, i64), (i64, i64)) {
    let p = s(k, 0, b"/dir/file");
    let st = scratch(0x800);
    assert_eq!(sys(k, nr::NEWFSTATAT, &[AT, p, st, 0]), Ok(0));
    let word = |off: u64| i64::from_le_bytes(k.peek(st + off, 8).try_into().unwrap());
    ((word(72), word(80)), (word(88), word(96)))
}

#[test]
fn utimes_takes_microseconds() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    let tv = scratch(0x400);
    let words: [i64; 4] = [1500, 250_000, 1600, 999_999];
    for (i, w) in words.iter().enumerate() {
        k.poke(tv + 8 * i as u64, &w.to_le_bytes());
    }
    assert_eq!(sys(k, nr::UTIMES, &[p, tv]), Ok(0));
    assert_eq!(times_of(k), ((1500, 250_000_000), (1600, 999_999_000)));
    k.poke(tv + 8, &1_000_000i64.to_le_bytes());
    assert_eq!(sys(k, nr::UTIMES, &[p, tv]), Err(Errno::EINVAL), "usec out of range");
    k.poke(tv + 8, &(-1i64).to_le_bytes());
    assert_eq!(sys(k, nr::UTIMES, &[p, tv]), Err(Errno::EINVAL), "negative usec");
    // futimesat relative to a directory; utime in whole seconds.
    k.poke(tv + 8, &5i64.to_le_bytes());
    let dir = s(k, 0x100, b"/dir");
    let dfd = sys(k, nr::OPENAT, &[AT, dir, 0o200000, 0]).unwrap();
    let rel = s(k, 0x200, b"file");
    assert_eq!(sys(k, nr::FUTIMESAT, &[dfd, rel, tv]), Ok(0));
    assert_eq!(times_of(k).0, (1500, 5000));
    let ub = scratch(0x600);
    k.poke(ub, &77i64.to_le_bytes());
    k.poke(ub + 8, &88i64.to_le_bytes());
    let p = s(k, 0, b"/dir/file");
    assert_eq!(sys(k, nr::UTIME, &[p, ub]), Ok(0));
    assert_eq!(times_of(k), ((77, 0), (88, 0)));
    let missing = s(k, 0x300, b"/dir/none");
    assert_eq!(sys(k, nr::UTIMES, &[missing, 0]), Err(Errno::ENOENT));
}

// ---- reads and writes at a partly bad buffer --------------------------------

#[test]
fn read_into_a_partly_mapped_buffer_is_short_and_keeps_the_offset_honest() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    let fd = sys(k, nr::OPENAT, &[AT, p, 0, 0]).unwrap();
    // "contents" is 8 bytes; 3 of them fit before the hole.
    assert_eq!(sys(k, nr::READ, &[fd, DATA_END - 3, 8]), Ok(3));
    assert_eq!(k.peek(DATA_END - 3, 3), b"con");
    assert_eq!(sys(k, nr::LSEEK, &[fd, 0, 1]), Ok(3), "the offset moved by what was delivered");
    assert_eq!(sys(k, nr::READ, &[fd, UNMAPPED, 3]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::LSEEK, &[fd, 0, 1]), Ok(3), "nothing delivered: the offset is unchanged");
    assert_eq!(sys(k, nr::LSEEK, &[fd, 0, 2]), Ok(8));
    assert_eq!(sys(k, nr::READ, &[fd, UNMAPPED, 3]), Ok(0), "at the end, the buffer is not touched");
}

#[test]
fn writev_writes_the_buffers_before_a_bad_one() {
    let mut e = env();
    let k = &mut e.k;
    let p = s(k, 0, b"/dir/file");
    let fd = sys(k, nr::OPENAT, &[AT, p, 0o1 | 0o1000, 0]).unwrap();
    let iov = scratch(0x400);
    let data = s(k, 0x500, b"x");
    for (i, w) in [data, 1, UNMAPPED, 4].iter().enumerate() {
        k.poke(iov + 8 * i as u64, &w.to_le_bytes());
    }
    assert_eq!(sys(k, nr::WRITEV, &[fd, iov, 2]), Ok(1));
    // write(2) from a buffer that runs into a hole writes what is readable.
    k.poke(DATA_END - 2, b"yz");
    assert_eq!(sys(k, nr::WRITE, &[fd, DATA_END - 2, 10]), Ok(2));
    assert_eq!(sys(k, nr::WRITE, &[fd, UNMAPPED, 10]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::FSTAT, &[fd, scratch(0x800)]), Ok(0));
    assert_eq!(i64::from_le_bytes(k.peek(scratch(0x800) + 48, 8).try_into().unwrap()), 3, "x, y, z");
}

// ---- RLIMIT_FSIZE --------------------------------------------------------------

#[test]
fn file_size_limit_cuts_writes_and_refuses_growth() {
    let mut e = env();
    let k = &mut e.k;
    let lim = scratch(0x300);
    k.poke(lim, &Rlimit { cur: 10, max: u64::MAX }.to_bytes());
    assert_eq!(sys(k, nr::SETRLIMIT, &[u64::from(RLIMIT_FSIZE), lim]), Ok(0));
    let p = s(k, 0, b"/dir/new");
    let fd = sys(k, nr::OPENAT, &[AT, p, 0o2 | 0o100 | 0o1000, 0o644]).unwrap();
    let buf = s(k, 0x100, b"0123456789abcdef");
    assert_eq!(sys(k, nr::WRITE, &[fd, buf, 6]), Ok(6));
    assert!(!k.me().pending(100).contains(crate::sys::fsize::SIGXFSZ));
    assert_eq!(sys(k, nr::WRITE, &[fd, buf, 8]), Ok(4), "cut at the limit");
    assert_eq!(sys(k, nr::WRITE, &[fd, buf, 1]), Err(Errno::EFBIG));
    assert!(k.me().pending(100).contains(crate::sys::fsize::SIGXFSZ), "SIGXFSZ raised");
    assert_eq!(sys(k, nr::PWRITE64, &[fd, buf, 1, 20]), Err(Errno::EFBIG));
    assert_eq!(sys(k, nr::PWRITE64, &[fd, buf, 4, 8]), Ok(2), "pwrite is cut too");
    assert_eq!(sys(k, nr::FTRUNCATE, &[fd, 11]), Err(Errno::EFBIG));
    assert_eq!(sys(k, nr::FTRUNCATE, &[fd, 10]), Ok(0));
    // Writes to other kinds of file are not limited.
    assert_eq!(sys(k, nr::WRITE, &[1, buf, 16]), Ok(16));
}

// ---- CPU-time clocks by id ------------------------------------------------------

#[test]
fn cpu_clock_ids_name_processes_and_threads() {
    let mut k = FakeKernel::new();
    // clockid_t is an int: the id is computed in 32 bits.
    let cpu = |pid: i32, kind: i32| (!pid) << 3 | kind;
    assert_eq!(decode(&k, id::CLOCK_MONOTONIC), None, "not an encoded id");
    assert_eq!(decode(&k, cpu(0, 2)), Some(Err(Errno::EINVAL)), "the fake accounts no CPU time");
    k.cpu = Some([CpuTimes { user_ns: 5, system_ns: 7 }, CpuTimes { user_ns: 50, system_ns: 70 }, CpuTimes::default()]);
    let all = |of| Some(Ok(CpuClock { of, counts: Counts::All }));
    assert_eq!(decode(&k, cpu(0, 2)), all(CpuTimesOf::Process), "the caller's process");
    assert_eq!(decode(&k, cpu(100, 2)), all(CpuTimesOf::Process), "its own pid");
    assert_eq!(decode(&k, cpu(0, 6)), all(CpuTimesOf::Thread(100)), "the caller's thread");
    assert_eq!(decode(&k, cpu(0, 1)), Some(Ok(CpuClock { of: CpuTimesOf::Process, counts: Counts::User })));
    assert_eq!(decode(&k, cpu(0, 3)), Some(Err(Errno::EINVAL)), "a file descriptor clock");
    assert_eq!(decode(&k, cpu(0, 7)), Some(Err(Errno::EINVAL)));
    assert_eq!(decode(&k, cpu(0x0fff_fff0, 2)), Some(Err(Errno::EINVAL)), "no such process");
    assert_eq!(decode(&k, cpu(0x0fff_fff0, 6)), Some(Err(Errno::EINVAL)), "no such thread");
    // clock_gettime and clock_getres on them.
    let out = scratch(0x100);
    assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[cpu(0, 2) as u64, out]), Ok(0));
    assert_eq!(k.peek_u64(out + 8), 120);
    assert_eq!(sys(&mut k, nr::CLOCK_GETTIME, &[cpu(0, 1) as u64, out]), Ok(0));
    assert_eq!(k.peek_u64(out + 8), 50, "user time only");
    assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[cpu(0, 6) as u64, out]), Ok(0));
    assert_eq!((k.peek_u64(out), k.peek_u64(out + 8)), (0, 1));
    assert_eq!(sys(&mut k, nr::CLOCK_GETRES, &[cpu(0x0fff_fff0, 2) as u64, out]), Err(Errno::EINVAL));
}
