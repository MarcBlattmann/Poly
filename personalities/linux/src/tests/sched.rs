//! sched_yield, sched_setaffinity, sched_getaffinity.

use super::fake::{DATA, FakeKernel, UNMAPPED, sys};
use crate::{Errno, nr};

const OUT: u64 = DATA + 0x300;

#[test]
fn yield_gives_up_the_cpu() {
    let mut k = FakeKernel::new();
    assert_eq!(sys(&mut k, nr::SCHED_YIELD, &[]), Ok(0));
    assert_eq!(k.yields, 1);
}

#[test]
fn get_affinity_returns_the_mask_and_its_size() {
    let mut k = FakeKernel::new();
    k.poke(OUT, &[0xff; 16]);
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[0, 16, OUT]), Ok(8));
    assert_eq!(k.peek(OUT, 16), [1, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    let tid = k.tid as u64;
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[tid, 8, OUT]), Ok(8));
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[0, 4, OUT]), Err(Errno::EINVAL), "smaller than the kernel's mask");
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[0, 8, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[4242, 8, OUT]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[(-5i64) as u64, 8, OUT]), Err(Errno::ESRCH));
}

#[test]
fn set_affinity_reads_the_mask_and_checks_it() {
    let mut k = FakeKernel::new();
    k.cpus = 4;
    // CPUs 1 and 3, in a 16-byte mask with a bit far beyond the system's.
    let mut mask = [0u8; 16];
    mask[0] = 0b1010;
    mask[15] = 0x80;
    k.poke(OUT, &mask);
    assert_eq!(sys(&mut k, nr::SCHED_SETAFFINITY, &[0, 16, OUT]), Ok(0));
    assert_eq!(k.affinity[&k.tid], 0b1010);
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[0, 8, OUT + 32]), Ok(8));
    assert_eq!(k.peek(OUT + 32, 1), [0b1010]);
    // A short mask counts: one byte names CPU 2.
    k.poke(OUT, &[0b100]);
    assert_eq!(sys(&mut k, nr::SCHED_SETAFFINITY, &[0, 1, OUT]), Ok(0));
    assert_eq!(k.affinity[&k.tid], 0b100);
    // No CPU the system has, an empty mask, a bad pointer, no such thread.
    k.poke(OUT, &[0b1_0000]);
    assert_eq!(sys(&mut k, nr::SCHED_SETAFFINITY, &[0, 1, OUT]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_SETAFFINITY, &[0, 0, OUT]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_SETAFFINITY, &[0, 8, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SCHED_SETAFFINITY, &[4242, 8, OUT]), Err(Errno::ESRCH));
    assert_eq!(k.affinity[&k.tid], 0b100, "failed calls change nothing");
}
