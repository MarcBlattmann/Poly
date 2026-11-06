//! The NUMA calls (`sys::numa`), remap_file_pages(2),
//! process_madvise(2) and process_mrelease(2) (`sys::procmem`) against
//! the fake kernel.

use super::fake::*;
use crate::abi::PAGE_SIZE;
use crate::errno::Errno;
use crate::nr;
use crate::sys::numa::*;

const MASK: u64 = DATA + 0x100;
const MODE: u64 = DATA + 0x200;
const PAGES: u64 = DATA + 0x300;
const STATUS: u64 = DATA + 0x400;
const NODES: u64 = DATA + 0x500;
/// A mapping of 6 pages for the policies.
const AREA: u64 = 0x80_0000;

fn put_mask(k: &mut FakeKernel, word: u64) -> u64 {
    k.poke(MASK, &word.to_le_bytes());
    MASK
}

fn policy(k: &mut FakeKernel, addr: u64, flags: u64) -> Result<(i32, u64), Errno> {
    k.poke(MODE, &(-7i32).to_le_bytes());
    k.poke(MASK, &0xeeu64.to_le_bytes());
    sys(k, nr::GET_MEMPOLICY, &[MODE, MASK, 64, addr, flags])?;
    Ok((k.peek_u32(MODE) as i32, k.peek_u64(MASK)))
}

fn mbind(k: &mut FakeKernel, addr: u64, len: u64, mode: u64, mask: u64, flags: u64) -> Result<u64, Errno> {
    let m = put_mask(k, mask);
    sys(k, nr::MBIND, &[addr, len, mode, m, 64, flags])
}

#[test]
fn set_mempolicy_reads_back() {
    let mut k = FakeKernel::new();
    assert_eq!(policy(&mut k, 0, 0), Ok((0, 0)), "MPOL_DEFAULT at start");
    let m = put_mask(&mut k, 3);
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY, &[0x8002, m, 64]), Ok(0));
    assert_eq!(policy(&mut k, 0, 0), Ok((0x8002, 3)));
    let m = put_mask(&mut k, 3);
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY, &[2, m, 64]), Ok(0));
    assert_eq!(policy(&mut k, 0, 0), Ok((2, 1)));
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY, &[1, 0, 0]), Ok(0));
    assert_eq!(policy(&mut k, 0, 0), Ok((4, 0)), "PREFERRED without a mask is LOCAL");
    // Failures keep the policy.
    let m = put_mask(&mut k, 2);
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY, &[2, m, 64]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY, &[3, UNMAPPED, 64]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY, &[2, MASK, 65536]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY, &[2, MASK, 1]), Err(Errno::EINVAL), "maxnode 1: no bits");
    assert_eq!(policy(&mut k, 0, 0), Ok((4, 0)));
    // A new thread inherits it.
    k.add_thread(101);
    k.tid = 101;
    assert_eq!(policy(&mut k, 0, 0), Ok((4, 0)));
}

#[test]
fn get_mempolicy_flags() {
    let mut k = FakeKernel::new();
    k.poke(MASK, &[0xee; 16]);
    assert_eq!(sys(&mut k, nr::GET_MEMPOLICY, &[MODE, MASK, 64, 0, MPOL_F_MEMS_ALLOWED]), Ok(0));
    assert_eq!((k.peek_u64(MASK), k.peek_u64(MASK + 8)), (1, 0xeeee_eeee_eeee_eeee));
    for (addr, flags) in [(0, MPOL_F_NODE), (DATA, 0), (0, 8), (DATA, MPOL_F_MEMS_ALLOWED | MPOL_F_ADDR)] {
        assert_eq!(sys(&mut k, nr::GET_MEMPOLICY, &[MODE, 0, 0, addr, flags]), Err(Errno::EINVAL), "{flags}");
    }
    assert_eq!(sys(&mut k, nr::GET_MEMPOLICY, &[MODE, MASK, 0, 0, 0]), Err(Errno::EINVAL), "maxnode 0");
    assert_eq!(sys(&mut k, nr::GET_MEMPOLICY, &[MODE, MASK, 32770, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::GET_MEMPOLICY, &[MODE, 0, 0, UNMAPPED, MPOL_F_ADDR]), Err(Errno::EFAULT));
    assert_eq!(policy(&mut k, DATA + 5, MPOL_F_ADDR | MPOL_F_NODE), Ok((0, 0)));
    let m = put_mask(&mut k, 1);
    sys(&mut k, nr::SET_MEMPOLICY, &[3, m, 64]).unwrap();
    assert_eq!(policy(&mut k, 0, MPOL_F_NODE), Ok((0, 1)), "the next interleave node");
    assert_eq!(policy(&mut k, DATA, MPOL_F_ADDR), Ok((0, 0)), "a mapping without a policy");
}

#[test]
fn mbind_sets_range_policies() {
    let mut k = FakeKernel::new();
    k.map_range(AREA, 6 * PAGE_SIZE, RW);
    assert_eq!(mbind(&mut k, AREA, 5 * PAGE_SIZE, 2, 1, 0), Ok(0));
    assert_eq!(mbind(&mut k, AREA + 2 * PAGE_SIZE, 1, 3, 1, 0), Ok(0), "len 1 is one page");
    let modes: [i32; 6] = core::array::from_fn(|i| policy(&mut k, AREA + i as u64 * PAGE_SIZE, MPOL_F_ADDR).unwrap().0);
    assert_eq!(modes, [2, 2, 3, 2, 2, 0]);
    assert_eq!(mbind(&mut k, AREA + 1, PAGE_SIZE, 2, 1, 0), Err(Errno::EINVAL));
    assert_eq!(mbind(&mut k, AREA, 0, 3, 1, 0), Ok(0));
    assert_eq!(mbind(&mut k, AREA, PAGE_SIZE, 2, 1, 8), Err(Errno::EINVAL));
    assert_eq!(mbind(&mut k, AREA, PAGE_SIZE, 2, 1, MPOL_MF_MOVE_ALL), Err(Errno::EPERM));
    assert_eq!(mbind(&mut k, AREA, 7 * PAGE_SIZE, 2, 1, 0), Err(Errno::EFAULT), "into a hole");
    assert_eq!(mbind(&mut k, 0u64.wrapping_sub(PAGE_SIZE), PAGE_SIZE, 2, 1, 0), Err(Errno::EINVAL));
    assert_eq!(policy(&mut k, AREA, MPOL_F_ADDR), Ok((2, 1)), "failures change nothing");
    // set_mempolicy_home_node walks the ranges.
    let hn = |k: &mut FakeKernel, at: u64, len: u64| sys(k, nr::SET_MEMPOLICY_HOME_NODE, &[at, len, 0, 0]);
    assert_eq!(hn(&mut k, AREA, PAGE_SIZE), Ok(0));
    assert_eq!(hn(&mut k, AREA, 3 * PAGE_SIZE), Err(Errno::EOPNOTSUPP));
    assert_eq!(hn(&mut k, AREA + 5 * PAGE_SIZE, PAGE_SIZE), Err(Errno::ENOENT));
    assert_eq!(hn(&mut k, AREA, 0), Ok(0));
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY_HOME_NODE, &[AREA, PAGE_SIZE, 1, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SET_MEMPOLICY_HOME_NODE, &[AREA, PAGE_SIZE, 0, 1]), Err(Errno::EINVAL));
    // MPOL_DEFAULT removes; munmap forgets.
    assert_eq!(sys(&mut k, nr::MBIND, &[AREA, PAGE_SIZE, 0, 0, 0, 0]), Ok(0));
    assert_eq!(policy(&mut k, AREA, MPOL_F_ADDR), Ok((0, 0)));
    assert_eq!(sys(&mut k, nr::MUNMAP, &[AREA + PAGE_SIZE, PAGE_SIZE]), Ok(0));
    k.map_range(AREA + PAGE_SIZE, PAGE_SIZE, RW);
    assert_eq!(policy(&mut k, AREA + PAGE_SIZE, MPOL_F_ADDR), Ok((0, 0)));
    assert_eq!(policy(&mut k, AREA + 3 * PAGE_SIZE, MPOL_F_ADDR), Ok((2, 1)));
}

#[test]
fn move_pages_reports_and_checks_nodes() {
    let mut k = FakeKernel::new();
    let pages = [DATA, DATA + PAGE_SIZE + 100, UNMAPPED];
    for (i, p) in pages.iter().enumerate() {
        k.poke(PAGES + 8 * i as u64, &p.to_le_bytes());
    }
    k.poke(STATUS, &[0x7f; 12]);
    assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[0, 3, PAGES, 0, STATUS, 0]), Ok(0));
    assert_eq!([k.peek_u32(STATUS), k.peek_u32(STATUS + 4), k.peek_u32(STATUS + 8)], [0, 0, (-14i32) as u32]);
    k.poke(NODES, &[0, 0, 0, 0, 1, 0, 0, 0]);
    k.poke(STATUS, &[0x7f; 12]);
    assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[0, 2, PAGES, NODES, STATUS, 0]), Err(Errno::ENODEV));
    assert_eq!(k.peek_u32(STATUS), 0x7f7f_7f7f, "status untouched");
    assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[0, 1, PAGES, NODES, STATUS, MPOL_MF_MOVE]), Ok(0));
    for flags in [MPOL_MF_STRICT, 8] {
        assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[0, 1, PAGES, 0, STATUS, flags]), Err(Errno::EINVAL));
    }
    assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[0, 1, PAGES, 0, STATUS, MPOL_MF_MOVE_ALL]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[0, 0, 0, 0, 0, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[555, 1, PAGES, 0, STATUS, 0]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[0, 1, UNMAPPED, 0, STATUS, 0]), Err(Errno::EFAULT));
    // Another user's process.
    k.add_process(200, 0, 200, 200);
    assert_eq!(sys(&mut k, nr::MOVE_PAGES, &[200, 1, PAGES, 0, STATUS, 0]), Err(Errno::EPERM));
}

#[test]
fn migrate_pages_checks() {
    let mut k = FakeKernel::new();
    let m = put_mask(&mut k, 1);
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[0, 64, m, m]), Ok(0));
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[0, 0, m, m]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[0, 65536, m, m]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[0, 64, 0, 0]), Err(Errno::EINVAL), "no new nodes");
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[0, 64, UNMAPPED, m]), Err(Errno::EFAULT));
    let two = DATA + 0x180;
    k.poke(two, &2u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[0, 64, m, two]), Err(Errno::EPERM), "node 1, unprivileged");
    k.add_process(200, 0, 200, 200);
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[200, 64, m, m]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[555, 64, m, m]), Err(Errno::ESRCH));
    k.creds_mut().caps = crate::cred::Caps::ROOT;
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[0, 64, m, two]), Err(Errno::EINVAL), "node 1, privileged");
    assert_eq!(sys(&mut k, nr::MIGRATE_PAGES, &[200, 64, m, m]), Ok(0));
}

#[test]
fn procmem_argument_checks() {
    let mut k = FakeKernel::new();
    // remap_file_pages: prot, size, a private mapping.
    assert_eq!(sys(&mut k, nr::REMAP_FILE_PAGES, &[DATA, PAGE_SIZE, 1, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::REMAP_FILE_PAGES, &[DATA, 100, 0, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::REMAP_FILE_PAGES, &[DATA, PAGE_SIZE, 0, u64::MAX, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::REMAP_FILE_PAGES, &[DATA, PAGE_SIZE, 0, 0, 0]), Err(Errno::EINVAL), "private");
    assert_eq!(sys(&mut k, nr::REMAP_FILE_PAGES, &[UNMAPPED, PAGE_SIZE, 0, 0, 0]), Err(Errno::EINVAL));
    // process_mrelease: flags first, then the descriptor.
    assert_eq!(sys(&mut k, nr::PROCESS_MRELEASE, &[99, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::PROCESS_MRELEASE, &[(-1i64) as u64, 0]), Err(Errno::EBADF));
    // process_madvise: flags, vlen, the array, then the descriptor.
    assert_eq!(sys(&mut k, nr::PROCESS_MADVISE, &[0, PAGES, 1, 3, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::PROCESS_MADVISE, &[0, PAGES, 1025, 3, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::PROCESS_MADVISE, &[0, UNMAPPED, 1, 3, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::PROCESS_MADVISE, &[(-1i64) as u64, PAGES, 1, 3, 0]), Err(Errno::EBADF));
}
