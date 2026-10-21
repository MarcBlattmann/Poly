//! System V shared memory: shmget(2), shmat(2), shmdt(2), shmctl(2)
//! (`specs/linux/x86_64/shmget.yaml` and its visible tests).
//!
//! Each test runs in an IPC namespace of its own, so tests running at the
//! same time do not see each other's segments. The fake's attachments are
//! fake mappings that count themselves; their pages are not shared.

use super::fake::*;
use crate::abi::{PAGE_SIZE, prot};
use crate::cred::Creds;
use crate::ipc::shm::*;
use crate::kernel::{Backing, LinuxKernel, MapHints, MapRequest, Placement, Prot, UserPtr};
use crate::ns::IpcNs;
use crate::{Errno, Linux, nr};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

/// The fake's attachments: address, memory, length.
#[derive(Default)]
pub struct FakeShm {
    pub attached: BTreeMap<u64, (Arc<dyn ShmMemory>, u64)>,
    serials: u64,
}

#[derive(Debug)]
struct Memory {
    serial: u64,
    attachments: AtomicU64,
}

impl ShmMemory for Memory {
    fn attachments(&self) -> u64 {
        self.attachments.load(Ordering::Relaxed)
    }
    fn serial(&self) -> u64 {
        self.serial
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn memory(m: &Arc<dyn ShmMemory>) -> &Memory {
    m.as_any().downcast_ref::<Memory>().expect("the fake's memory")
}

pub fn create(k: &mut FakeKernel, len: u64) -> Result<Arc<dyn ShmMemory>, Errno> {
    assert!(len > 0 && len.is_multiple_of(PAGE_SIZE), "whole pages: {len}");
    k.shm.serials += 1;
    Ok(Arc::new(Memory { serial: k.shm.serials, attachments: AtomicU64::new(0) }))
}

pub fn attach(
    k: &mut FakeKernel,
    m: &Arc<dyn ShmMemory>,
    addr: u64,
    placement: Placement,
    len: u64,
    prot: Prot,
) -> Result<u64, Errno> {
    let r = MapRequest {
        addr,
        len,
        prot,
        placement,
        shared: true,
        backing: Backing::Anonymous,
        hints: MapHints::default(),
    };
    let at = LinuxKernel::mmap(k, &r).map_err(|e| if e == Errno::EEXIST { Errno::EINVAL } else { e })?;
    memory(m).attachments.fetch_add(1, Ordering::Relaxed);
    k.shm.attached.insert(at, (m.clone(), len));
    Ok(at)
}

pub fn detach(k: &mut FakeKernel, addr: u64) -> Result<u64, Errno> {
    let (m, len) = k.shm.attached.remove(&addr).ok_or(Errno::EINVAL)?;
    LinuxKernel::munmap(k, addr, len)?;
    memory(&m).attachments.fetch_sub(1, Ordering::Relaxed);
    Ok(m.serial())
}

pub fn detach_all(k: &mut FakeKernel) -> Vec<u64> {
    let addrs: Vec<u64> = k.shm.attached.keys().copied().collect();
    addrs.into_iter().map(|a| detach(k, a).unwrap()).collect()
}

/// A fake process in an IPC namespace of its own (euid 1001, egid 2001,
/// no capabilities).
fn fresh() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.me().ns.ipc = Some(IpcNs::create(&None));
    k
}

fn shmget(k: &mut FakeKernel, key: i64, size: u64, flags: i64) -> Result<i32, Errno> {
    sys(k, nr::SHMGET, &[key as u64, size, flags as u64]).map(|id| id as i32)
}

fn rm(k: &mut FakeKernel, id: i32) -> Result<u64, Errno> {
    sys(k, nr::SHMCTL, &[id as u64, u64::from(IPC_RMID), 0])
}

const BUF: u64 = DATA + 0x1000;

fn stat(k: &mut FakeKernel, id: i32) -> Result<ShmidDs, Errno> {
    sys(k, nr::SHMCTL, &[id as u64, u64::from(IPC_STAT), BUF])?;
    let mut b = [0u8; ShmidDs::SIZE];
    k.copy_from_user(UserPtr(BUF), &mut b).unwrap();
    let i32_at = |at: usize| i32::from_ne_bytes(b[at..at + 4].try_into().unwrap());
    let u64_at = |at: usize| u64::from_ne_bytes(b[at..at + 8].try_into().unwrap());
    Ok(ShmidDs {
        key: i32_at(0),
        uid: i32_at(4) as u32,
        gid: i32_at(8) as u32,
        cuid: i32_at(12) as u32,
        cgid: i32_at(16) as u32,
        mode: u16::from_ne_bytes([b[20], b[21]]),
        seq: u16::from_ne_bytes([b[24], b[25]]),
        segsz: u64_at(48),
        atime: u64_at(56) as i64,
        dtime: u64_at(64) as i64,
        ctime: u64_at(72) as i64,
        cpid: i32_at(80),
        lpid: i32_at(84),
        nattch: u64_at(88),
    })
}

fn shmat(k: &mut FakeKernel, id: i32, addr: u64, flags: u32) -> Result<u64, Errno> {
    sys(k, nr::SHMAT, &[id as u64, addr, u64::from(flags)])
}

fn shmdt(k: &mut FakeKernel, addr: u64) -> Result<u64, Errno> {
    sys(k, nr::SHMDT, &[addr])
}

#[test]
fn ids_are_distinct_and_removed_ones_are_gone() {
    let mut k = fresh();
    let ids: Vec<i32> = (0..64).map(|_| shmget(&mut k, 0, 16, 0o600).unwrap()).collect();
    for (i, id) in ids.iter().enumerate() {
        assert!(*id >= 0 && !ids[..i].contains(id));
    }
    for id in &ids {
        assert_eq!(rm(&mut k, *id), Ok(0));
    }
    let a = shmget(&mut k, 0, 16, 0o600).unwrap();
    rm(&mut k, a).unwrap();
    let b = shmget(&mut k, 0, 16, 0o600).unwrap();
    assert_ne!(a, b, "the id just removed is not the next");
    rm(&mut k, b).unwrap();

    let id = shmget(&mut k, 0x5000_0002, 16, 0o600 | IPC_CREAT as i64).unwrap();
    assert_eq!(rm(&mut k, id), Ok(0));
    assert_eq!(stat(&mut k, id), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0x5000_0002, 16, 0o600), Err(Errno::ENOENT));
    let id2 = shmget(&mut k, 0x5000_0002, 16, 0o600 | IPC_CREAT as i64).unwrap();
    assert_ne!(id2, id);
    assert!(k.shm.attached.is_empty() && k.files.len() == 3, "no attachment, no descriptor");
}

#[test]
fn new_segment_stat() {
    let mut k = fresh();
    let id = shmget(&mut k, -7, 5000, 0o640 | IPC_CREAT as i64).unwrap();
    let ds = stat(&mut k, id).unwrap();
    let expected = ShmidDs {
        key: -7,
        uid: 1001,
        gid: 2001,
        cuid: 1001,
        cgid: 2001,
        mode: 0o640,
        seq: 0,
        segsz: 5000,
        atime: 0,
        dtime: 0,
        ctime: k.realtime.sec,
        cpid: 100,
        lpid: 0,
        nattch: 0,
    };
    assert_eq!(ds, expected);
    assert!(k.maps.is_empty(), "shmget never attaches");
}

#[test]
fn key_and_flags_use_the_low_32_bits() {
    let mut k = fresh();
    // A key whose low half is 0 is IPC_PRIVATE.
    let a = shmget(&mut k, 0x1_0000_0000, 100, 0o600).unwrap();
    let b = shmget(&mut k, 0x1_0000_0000, 100, 0o600).unwrap();
    assert_ne!(a, b);
    for flags in [0o600 | 0xffff_0000_0000, 0o600 | SHM_NORESERVE as i64, 0o600 | 0x10_0000, 0o600 | 0x8000_0000] {
        let id = shmget(&mut k, 0, 100, flags).unwrap();
        assert_eq!(stat(&mut k, id).unwrap().mode, 0o600, "{flags:#x}");
    }
    let id = shmget(&mut k, 0, 100, 0o754 | (IPC_CREAT | IPC_EXCL) as i64).unwrap();
    assert_eq!(stat(&mut k, id).unwrap().mode, 0o754);
}

#[test]
fn size_errors_make_nothing() {
    let mut k = fresh();
    let creat = 0o600 | IPC_CREAT as i64;
    assert_eq!(shmget(&mut k, 0, 0, 0o600), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0, 0, creat | IPC_EXCL as i64), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0, u64::MAX, 0o600), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0, 1 << 63, 0o600), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0x5080_0000, 0, creat), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0x5080_0000, u64::MAX, creat), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0x5080_0000, 100, 0), Err(Errno::ENOENT));
    for size in [1, 4096, 1 << 20] {
        assert!(shmget(&mut k, 0, size, 0o600).is_ok());
    }
}

#[test]
fn more_than_ram_and_swap_is_enomem_unless_noreserve() {
    let mut k = fresh();
    k.sysinfo = Some(crate::fs::SysInfo { totalram: 1 << 30, totalswap: 0, mem_unit: 1, ..Default::default() });
    assert_eq!(shmget(&mut k, 0, 1 << 40, 0o600), Err(Errno::ENOMEM));
    assert!(shmget(&mut k, 0, 1 << 40, 0o600 | SHM_NORESERVE as i64).is_ok());
    assert!(shmget(&mut k, 0, 1 << 20, 0o600).is_ok());
}

#[test]
fn existing_segment_sizes_and_error_order() {
    let mut k = fresh();
    let creat = IPC_CREAT as i64;
    let id = shmget(&mut k, 0x5140_0001, 5000, 0o600 | creat).unwrap();
    for size in [5000, 1, 0] {
        assert_eq!(shmget(&mut k, 0x5140_0001, size, 0), Ok(id));
    }
    assert_eq!(shmget(&mut k, 0x5140_0001, 5001, 0), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0x5140_0001, 8192, 0o600 | creat), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0x5140_0001, 8192, 0o600 | creat | IPC_EXCL as i64), Err(Errno::EEXIST));
    assert_eq!(stat(&mut k, id).unwrap().segsz, 5000);

    shmget(&mut k, 0x5180_0001, 100, 0o400 | creat).unwrap();
    assert_eq!(shmget(&mut k, 0x5180_0001, 200, 0o600), Err(Errno::EINVAL));
    assert_eq!(shmget(&mut k, 0x5180_0001, 200, 0o600 | creat | IPC_EXCL as i64), Err(Errno::EEXIST));
}

#[test]
fn access_checks_for_a_user_and_for_root() {
    for root in [false, true] {
        let mut k = fresh();
        if root {
            *k.creds_mut() = Creds::root();
        }
        let denied = |id: i32| if root { Ok(id) } else { Err(Errno::EACCES) };
        let excl = (IPC_CREAT | IPC_EXCL) as i64;
        let key = 0x5040_0001;
        let id = shmget(&mut k, key, 100, 0o640 | excl).unwrap();
        for ok in [0, 0o600, 0o400, 0o040, 0o020, 0o006] {
            assert_eq!(shmget(&mut k, key, 0, ok), Ok(id), "{ok:o}");
        }
        for no in [0o100, 0o001, 0o777, 0o700 | IPC_CREAT as i64] {
            assert_eq!(shmget(&mut k, key, 0, no), denied(id), "{no:o}");
        }
        let ro = shmget(&mut k, key + 1, 100, 0o400 | excl).unwrap();
        assert_eq!(shmget(&mut k, key + 1, 0, 0o444), Ok(ro));
        assert_eq!(shmget(&mut k, key + 1, 0, 0o200), denied(ro));
        assert_eq!(shmget(&mut k, key + 1, 0, 0o002), denied(ro));
        let zero = shmget(&mut k, key + 2, 100, excl).unwrap();
        assert_eq!(shmget(&mut k, key + 2, 0, 0), Ok(zero));
        assert_eq!(shmget(&mut k, key + 2, 0, 0o400), denied(zero));
        // IPC_STAT needs read permission too.
        assert_eq!(stat(&mut k, zero).map(|d| d.mode), if root { Ok(0) } else { Err(Errno::EACCES) });
    }
}

#[test]
fn attach_and_detach_are_counted_and_recorded() {
    let mut k = fresh();
    let id = shmget(&mut k, 0, 4097, 0o600).unwrap();
    k.realtime.sec += 5;
    let a = shmat(&mut k, id, 0, 0).unwrap();
    let b = shmat(&mut k, id, 0, 0).unwrap();
    assert_ne!(a, b);
    let ds = stat(&mut k, id).unwrap();
    assert_eq!((ds.nattch, ds.lpid, ds.atime, ds.dtime), (2, 100, k.realtime.sec, 0));
    let r = k.maps.last().unwrap();
    assert_eq!((r.len, r.shared, r.prot), (2 * PAGE_SIZE, true, Prot(prot::PROT_READ | prot::PROT_WRITE)));
    k.realtime.sec += 5;
    assert_eq!(shmdt(&mut k, a), Ok(0));
    assert_eq!(shmdt(&mut k, a), Err(Errno::EINVAL), "nothing attached there any more");
    assert_eq!(shmdt(&mut k, b + 1), Err(Errno::EINVAL));
    let ds = stat(&mut k, id).unwrap();
    assert_eq!((ds.nattch, ds.dtime), (1, k.realtime.sec));
    shmdt(&mut k, b).unwrap();
    assert_eq!(stat(&mut k, id).unwrap().nattch, 0);
}

#[test]
fn removed_while_attached() {
    let mut k = fresh();
    let key = 0x5140_0002;
    let id = shmget(&mut k, key, 4096, 0o600 | (IPC_CREAT | IPC_EXCL) as i64).unwrap();
    let p = shmat(&mut k, id, 0, 0).unwrap();
    assert_eq!(rm(&mut k, id), Ok(0));
    assert_eq!(shmget(&mut k, key, 0, 0), Err(Errno::ENOENT));
    let ds = stat(&mut k, id).unwrap();
    assert_eq!((ds.key, ds.mode, ds.nattch), (0, SHM_DEST | 0o600, 1));
    let id2 = shmget(&mut k, key, 100, 0o600 | (IPC_CREAT | IPC_EXCL) as i64).unwrap();
    assert_ne!(id2, id);
    assert_eq!(shmget(&mut k, key, 0, 0), Ok(id2));
    shmdt(&mut k, p).unwrap();
    assert_eq!(stat(&mut k, id), Err(Errno::EINVAL), "gone with its last detach");
    assert_eq!(shmget(&mut k, key, 0, 0), Ok(id2));
    rm(&mut k, id2).unwrap();
    assert_eq!(shmget(&mut k, key, 0, 0), Err(Errno::ENOENT));
}

#[test]
fn exit_detaches_everything() {
    let mut k = fresh();
    let id = shmget(&mut k, 0, 64, 0o600).unwrap();
    shmat(&mut k, id, 0, 0).unwrap();
    shmat(&mut k, id, 0, SHM_RDONLY).unwrap();
    k.realtime.sec += 1;
    Linux::process_exit(&mut k);
    let ds = stat(&mut k, id).unwrap();
    assert_eq!((ds.nattch, ds.lpid, ds.dtime), (0, 100, k.realtime.sec));
    // A removed segment goes with the exit's detach.
    shmat(&mut k, id, 0, 0).unwrap();
    rm(&mut k, id).unwrap();
    Linux::process_exit(&mut k);
    assert_eq!(stat(&mut k, id), Err(Errno::EINVAL));
}

#[test]
fn shmat_addresses_and_flags() {
    let mut k = fresh();
    let id = shmget(&mut k, 0, 100, 0o700).unwrap();
    let at = 0x6000_0000;
    assert_eq!(shmat(&mut k, id, 0, SHM_REMAP), Err(Errno::EINVAL));
    assert_eq!(shmat(&mut k, id, at + 12, 0), Err(Errno::EINVAL), "not aligned");
    assert_eq!(shmat(&mut k, id, at + 12, SHM_RND), Ok(at));
    assert_eq!(shmat(&mut k, id, at, 0), Err(Errno::EINVAL), "taken");
    assert_eq!(shmat(&mut k, id, at, SHM_REMAP), Ok(at));
    assert_eq!(k.maps.last().unwrap().placement, Placement::Fixed);
    assert_eq!(shmat(&mut k, id + 1000, 0, 0), Err(Errno::EINVAL));
    assert_eq!(shmat(&mut k, -1, 0, 0), Err(Errno::EINVAL));
    shmat(&mut k, id, 0, SHM_RDONLY | SHM_EXEC).unwrap();
    assert_eq!(k.maps.last().unwrap().prot, Prot(prot::PROT_READ | prot::PROT_EXEC));

    let ro = shmget(&mut k, 0, 100, 0o400).unwrap();
    assert_eq!(shmat(&mut k, ro, 0, 0), Err(Errno::EACCES));
    assert!(shmat(&mut k, ro, 0, SHM_RDONLY).is_ok());
    assert_eq!(shmat(&mut k, ro, 0, SHM_RDONLY | SHM_EXEC), Err(Errno::EACCES));
}

#[test]
fn shmctl_set_and_permissions() {
    let mut k = fresh();
    let id = shmget(&mut k, 0, 100, 0o600).unwrap();
    let mut ds = stat(&mut k, id).unwrap();
    ds.mode = 0o7644;
    ds.uid = 1001;
    ds.gid = 3000;
    k.copy_to_user(UserPtr(BUF), &ds.to_bytes()).unwrap();
    k.realtime.sec += 9;
    assert_eq!(sys(&mut k, nr::SHMCTL, &[id as u64, u64::from(IPC_SET), BUF]), Ok(0));
    let after = stat(&mut k, id).unwrap();
    assert_eq!((after.mode, after.uid, after.gid, after.cgid, after.ctime), (0o644, 1001, 3000, 2001, k.realtime.sec));
    assert_eq!(sys(&mut k, nr::SHMCTL, &[id as u64, u64::from(IPC_STAT | IPC_64), BUF]), Ok(0));
    assert_eq!(sys(&mut k, nr::SHMCTL, &[id as u64, 99, BUF]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SHMCTL, &[id as u64, u64::from(IPC_STAT), 0]), Err(Errno::EFAULT));

    // Someone else may look (mode 0644) but not change or remove it.
    *k.creds_mut() = Creds::user(4000, 4000);
    assert!(stat(&mut k, id).is_ok());
    assert_eq!(sys(&mut k, nr::SHMCTL, &[id as u64, u64::from(IPC_SET), BUF]), Err(Errno::EPERM));
    assert_eq!(rm(&mut k, id), Err(Errno::EPERM));
    *k.creds_mut() = Creds::root();
    assert_eq!(rm(&mut k, id), Ok(0));
}

#[test]
fn namespaces_have_their_own_segments() {
    let mut k = fresh();
    let id = shmget(&mut k, 0x5200_0001, 100, 0o600 | IPC_CREAT as i64).unwrap();
    k.me().ns.ipc = Some(IpcNs::create(&None));
    assert_eq!(shmget(&mut k, 0x5200_0001, 0, 0), Err(Errno::ENOENT));
    assert_eq!(stat(&mut k, id), Err(Errno::EINVAL));
}
