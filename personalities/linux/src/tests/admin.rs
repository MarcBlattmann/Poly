//! The administration calls (mount, umount2, mknod, chroot, sethostname,
//! setdomainname, reboot) and priorities and scheduling, against a
//! recording [`LinuxAdmin`]. (sendfile, ADR 0051, is in `sendfile.rs`.)

extern crate std;

use super::fake::*;
use crate::admin::{self, FsType, LinuxAdmin, MountFlags, MountRequest, Reboot, RootOptions, SystemName};
use crate::cred::Creds;
use crate::sys::admin::{MAGIC1, MAGIC2};
use crate::sys::sched::{PRIO_PGRP, PRIO_PROCESS, PRIO_USER, SCHED_FIFO, SCHED_OTHER};
use crate::{Errno, nr};
use std::boxed::Box;
use std::string::String;
use std::sync::{Arc, Mutex};
use std::vec::Vec;

/// What reached the kernel.
#[derive(Debug, Default)]
struct Log {
    mounts: Vec<MountRequest>,
    umounts: Vec<(Vec<u8>, bool, bool)>,
    nodes: Vec<(i32, Vec<u8>, u32, u64)>,
    roots: Vec<Vec<u8>>,
    reboots: Vec<Reboot>,
    cad: Vec<bool>,
}

struct Recorder {
    log: Arc<Mutex<Log>>,
    host: Vec<u8>,
    domain: Vec<u8>,
}

impl LinuxAdmin for Recorder {
    fn mount(&mut self, req: &MountRequest) -> Result<(), Errno> {
        self.log.lock().unwrap().mounts.push(req.clone());
        Ok(())
    }
    fn umount(&mut self, target: &[u8], detach: bool, follow: bool) -> Result<(), Errno> {
        self.log.lock().unwrap().umounts.push((target.to_vec(), detach, follow));
        Ok(())
    }
    fn mknodat(&mut self, dirfd: i32, path: &[u8], mode: u32, dev: u64) -> Result<(), Errno> {
        self.log.lock().unwrap().nodes.push((dirfd, path.to_vec(), mode, dev));
        Ok(())
    }
    fn chroot(&mut self, path: &[u8]) -> Result<(), Errno> {
        self.log.lock().unwrap().roots.push(path.to_vec());
        Ok(())
    }
    fn system_name(&self, which: SystemName) -> Vec<u8> {
        match which {
            SystemName::Host => self.host.clone(),
            SystemName::Domain => self.domain.clone(),
        }
    }
    fn set_system_name(&mut self, which: SystemName, name: &[u8]) -> Result<(), Errno> {
        match which {
            SystemName::Host => self.host = name.to_vec(),
            SystemName::Domain => self.domain = name.to_vec(),
        }
        Ok(())
    }
    fn set_ctrl_alt_del(&mut self, reboots: bool) {
        self.log.lock().unwrap().cad.push(reboots);
    }
    fn reboot(&mut self, how: Reboot) -> Errno {
        self.log.lock().unwrap().reboots.push(how);
        // A kernel that could not do it.
        Errno::EIO
    }
}

/// A fake kernel with a recording admin service, called by root.
fn kernel() -> (FakeKernel, Arc<Mutex<Log>>) {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    let log = Arc::new(Mutex::new(Log::default()));
    k.admin = Some(Box::new(Recorder { log: Arc::clone(&log), host: b"poly".to_vec(), domain: b"(none)".to_vec() }));
    (k, log)
}

/// NUL-terminated strings in user memory, at DATA, DATA + 0x100, ...
fn strings(k: &mut FakeKernel, list: &[&[u8]]) -> Vec<u64> {
    list.iter()
        .enumerate()
        .map(|(i, s)| {
            let at = DATA + 0x100 * i as u64;
            let mut b = s.to_vec();
            b.push(0);
            k.poke(at, &b);
            at
        })
        .collect()
}

#[test]
fn mount_decodes_the_operation_from_the_flags() {
    let (mut k, log) = kernel();
    let p = strings(&mut k, &[b"tmpfs", b"/mnt", b"tmpfs", b"mode=1777,size=1m"]);
    let flags = admin::ms::NOSUID | admin::ms::NODEV | admin::ms::RELATIME;
    assert_eq!(sys(&mut k, nr::MOUNT, &[p[0], p[1], p[2], flags, p[3]]), Ok(0));
    // The old magic number in the top 16 bits is ignored.
    assert_eq!(sys(&mut k, nr::MOUNT, &[0, p[1], 0, admin::ms::MGC_VAL | admin::ms::REMOUNT | 1, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::MOUNT, &[p[1], p[1], 0, admin::ms::BIND | admin::ms::REC, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::MOUNT, &[0, p[1], 0, admin::ms::PRIVATE | admin::ms::REC, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::MOUNT, &[p[1], p[1], 0, admin::ms::MOVE, 0]), Ok(0));
    let mounts = std::mem::take(&mut log.lock().unwrap().mounts);
    assert_eq!(
        mounts,
        [
            MountRequest::New {
                source: b"tmpfs".to_vec(),
                target: b"/mnt".to_vec(),
                fstype: b"tmpfs".to_vec(),
                flags: MountFlags { no_suid: true, no_dev: true, ..MountFlags::default() },
                data: b"mode=1777,size=1m".to_vec(),
            },
            MountRequest::Remount {
                target: b"/mnt".to_vec(),
                flags: MountFlags { read_only: true, ..MountFlags::default() },
                data: Vec::new()
            },
            MountRequest::Bind {
                source: b"/mnt".to_vec(),
                target: b"/mnt".to_vec(),
                flags: MountFlags::default(),
                recursive: true
            },
            MountRequest::Propagation { target: b"/mnt".to_vec() },
            MountRequest::Move { source: b"/mnt".to_vec(), target: b"/mnt".to_vec() },
        ]
    );
    // Two propagation types at once; no type for a new mount; a bad pointer.
    let both = admin::ms::PRIVATE | admin::ms::SHARED;
    assert_eq!(sys(&mut k, nr::MOUNT, &[0, p[1], 0, both, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MOUNT, &[p[0], p[1], 0, 0, 0]), Err(Errno::ENODEV));
    assert_eq!(sys(&mut k, nr::MOUNT, &[p[0], UNMAPPED, p[2], 0, 0]), Err(Errno::EFAULT));
    assert!(log.lock().unwrap().mounts.is_empty());
}

#[test]
fn administration_needs_privileges() {
    let (mut k, log) = kernel();
    *k.creds_mut() = Creds::user(1000, 1000);
    let p = strings(&mut k, &[b"none", b"/mnt", b"tmpfs", b"newhost", b"/dev/null2"]);
    assert_eq!(sys(&mut k, nr::MOUNT, &[p[0], p[1], p[2], 0, 0]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::UMOUNT2, &[p[1], 0]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[p[3], 7]), Err(Errno::EPERM));
    assert_eq!(
        sys(&mut k, nr::REBOOT, &[MAGIC1.into(), MAGIC2[0].into(), admin::cmd::POWER_OFF.into(), 0]),
        Err(Errno::EPERM)
    );
    assert_eq!(sys(&mut k, nr::CHROOT, &[p[1]]), Err(Errno::EPERM));
    // Device nodes need CAP_MKNOD; FIFOs do not.
    assert_eq!(sys(&mut k, nr::MKNOD, &[p[4], 0o020666, 0x103]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::MKNOD, &[p[4], 0o010644, 0]), Ok(0));
    let log = log.lock().unwrap();
    assert!(log.mounts.is_empty() && log.umounts.is_empty() && log.reboots.is_empty() && log.roots.is_empty());
    assert_eq!(log.nodes.len(), 1);
}

#[test]
fn umount2_flags() {
    let (mut k, log) = kernel();
    let p = strings(&mut k, &[b"/mnt"]);
    assert_eq!(sys(&mut k, nr::UMOUNT2, &[p[0], 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::UMOUNT2, &[p[0], u64::from(admin::mnt::DETACH | admin::mnt::UMOUNT_NOFOLLOW)]), Ok(0));
    assert_eq!(sys(&mut k, nr::UMOUNT2, &[p[0], u64::from(admin::mnt::FORCE)]), Ok(0));
    assert_eq!(sys(&mut k, nr::UMOUNT2, &[p[0], 0x100]), Err(Errno::EINVAL));
    let expire_detach = u64::from(admin::mnt::EXPIRE | admin::mnt::DETACH);
    assert_eq!(sys(&mut k, nr::UMOUNT2, &[p[0], expire_detach]), Err(Errno::EINVAL));
    assert_eq!(
        log.lock().unwrap().umounts,
        [(b"/mnt".to_vec(), false, true), (b"/mnt".to_vec(), true, false), (b"/mnt".to_vec(), false, true)]
    );
}

#[test]
fn mknod_types_and_device_numbers() {
    let (mut k, log) = kernel();
    let p = strings(&mut k, &[b"node"]);
    // makedev(4, 64) in the system call's 32-bit encoding; a zero type is
    // a regular file.
    assert_eq!(sys(&mut k, nr::MKNODAT, &[(-100i64) as u64, p[0], 0o020660, 0x440]), Ok(0));
    assert_eq!(sys(&mut k, nr::MKNOD, &[p[0], 0o644, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::MKNOD, &[p[0], 0o040755, 0]), Err(Errno::EINVAL));
    let nodes = &log.lock().unwrap().nodes;
    assert_eq!(nodes[0], (-100, b"node".to_vec(), 0o020660, 0x440));
    assert_eq!((crate::fs::major(nodes[0].3), crate::fs::minor(nodes[0].3)), (4, 64));
    assert_eq!(nodes[1].2, 0o100644);
}

#[test]
fn host_and_domain_names() {
    let (mut k, _) = kernel();
    let p = strings(&mut k, &[b"busybox-host"]);
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[p[0], 12]), Ok(0));
    assert_eq!(sys(&mut k, nr::SETDOMAINNAME, &[p[0], 7]), Ok(0));
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[p[0], 65]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[p[0], u64::MAX]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SETHOSTNAME, &[UNMAPPED, 4]), Err(Errno::EFAULT));
    // uname reports them: struct utsname is six fields of 65 bytes.
    let buf = DATA + 0x1000;
    assert_eq!(sys(&mut k, nr::UNAME, &[buf]), Ok(0));
    let field = |k: &FakeKernel, i: u64| {
        let b = k.peek(buf + 65 * i, 65);
        String::from_utf8(b[..b.iter().position(|&c| c == 0).unwrap()].to_vec()).unwrap()
    };
    assert_eq!(field(&k, 1), "busybox-host");
    assert_eq!(field(&k, 5), "busybox");
}

#[test]
fn reboot_commands() {
    let (mut k, log) = kernel();
    let call = |k: &mut FakeKernel, magic: u32, magic2: u32, cmd: u32| {
        sys(k, nr::REBOOT, &[magic.into(), magic2.into(), cmd.into(), 0])
    };
    assert_eq!(call(&mut k, MAGIC1, MAGIC2[2], admin::cmd::CAD_OFF), Ok(0));
    assert_eq!(call(&mut k, MAGIC1, MAGIC2[3], admin::cmd::CAD_ON), Ok(0));
    // The kernel returns only when it failed.
    assert_eq!(call(&mut k, MAGIC1, MAGIC2[0], admin::cmd::POWER_OFF), Err(Errno::EIO));
    assert_eq!(call(&mut k, MAGIC1, MAGIC2[1], admin::cmd::HALT), Err(Errno::EIO));
    assert_eq!(call(&mut k, MAGIC1, MAGIC2[1], admin::cmd::RESTART), Err(Errno::EIO));
    assert_eq!(call(&mut k, 0xdead, MAGIC2[1], admin::cmd::RESTART), Err(Errno::EINVAL));
    assert_eq!(call(&mut k, MAGIC1, 1, admin::cmd::RESTART), Err(Errno::EINVAL));
    assert_eq!(call(&mut k, MAGIC1, MAGIC2[0], admin::cmd::KEXEC), Err(Errno::EINVAL));
    assert_eq!(call(&mut k, MAGIC1, MAGIC2[0], admin::cmd::SW_SUSPEND), Err(Errno::EINVAL));
    let log = log.lock().unwrap();
    assert_eq!(log.cad, [false, true]);
    assert_eq!(log.reboots, [Reboot::PowerOff, Reboot::Halt, Reboot::Restart]);
}

#[test]
fn without_the_service_the_calls_are_enosys() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    let p = strings(&mut k, &[b"/mnt", b"tmpfs"]);
    assert_eq!(sys(&mut k, nr::MOUNT, &[p[1], p[0], p[1], 0, 0]), Err(Errno::ENOSYS));
    assert_eq!(sys(&mut k, nr::UMOUNT2, &[p[0], 0]), Err(Errno::ENOSYS));
}

#[test]
fn file_system_types_and_options() {
    assert_eq!(FsType::from_name(b"tmpfs"), Ok(FsType::Tmpfs));
    assert_eq!(FsType::from_name(b"ramfs"), Ok(FsType::Tmpfs));
    assert_eq!(FsType::from_name(b"ext4"), Ok(FsType::Ext2));
    assert_eq!(FsType::from_name(b"btrfs"), Err(Errno::ENODEV));
    assert!(FsType::Ext2.needs_device() && !FsType::Proc.needs_device());
    let opts = admin::mount_options(FsType::Tmpfs, b"mode=0700,uid=5,gid=6,size=10%,nr_inodes=1k,,noswap").unwrap();
    assert_eq!(opts, RootOptions { mode: Some(0o700), uid: Some(5), gid: Some(6) });
    assert_eq!(admin::mount_options(FsType::Tmpfs, b"mode=9"), Err(Errno::EINVAL));
    assert_eq!(admin::mount_options(FsType::Tmpfs, b"size=lots"), Err(Errno::EINVAL));
    assert_eq!(admin::mount_options(FsType::Tmpfs, b"bogus"), Err(Errno::EINVAL));
    assert!(admin::mount_options(FsType::Devpts, b"gid=5,mode=620,ptmxmode=666,newinstance").is_ok());
    assert_eq!(admin::mount_options(FsType::Devpts, b"mode=x"), Err(Errno::EINVAL));
    assert!(admin::mount_options(FsType::Proc, b"hidepid=2,gid=10").is_ok());
    assert_eq!(admin::mount_options(FsType::Sysfs, b"anything"), Err(Errno::EINVAL));
    assert!(admin::mount_options(FsType::Ext2, b"errors=remount-ro,user_xattr").is_ok());
}

// ---- priorities and scheduling ------------------------------------------

/// The nice value of process `pid`'s main thread.
fn nice_of(k: &FakeKernel, pid: i32) -> i32 {
    k.procs[&pid].state.thread(pid).sched.nice
}

#[test]
fn nice_values_by_process_group_and_user() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    let me = k.pid;
    let pgid = k.procs[&me].info.pgid;
    k.add_process(200, 2000, pgid, 1);
    k.add_process(201, 2000, 999, 1);
    // getpriority returns 20 - nice.
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_PROCESS as u64, 0]), Ok(20));
    assert_eq!(sys(&mut k, nr::SETPRIORITY, &[PRIO_PROCESS as u64, 0, 5]), Ok(0));
    assert_eq!(k.thread_state().sched.nice, 5);
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_PROCESS as u64, 0]), Ok(15));
    // A group: the highest priority (lowest nice) of its members.
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_PGRP as u64, 0]), Ok(20));
    assert_eq!(sys(&mut k, nr::SETPRIORITY, &[PRIO_PGRP as u64, 0, (-3i64) as u64]), Ok(0));
    assert_eq!((k.thread_state().sched.nice, nice_of(&k, 200), nice_of(&k, 201)), (-3, -3, 0));
    // A user; values are clamped.
    assert_eq!(sys(&mut k, nr::SETPRIORITY, &[PRIO_USER as u64, 2000, 100]), Ok(0));
    assert_eq!((nice_of(&k, 200), nice_of(&k, 201), k.thread_state().sched.nice), (19, 19, -3));
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_USER as u64, 2000]), Ok(1));
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_PROCESS as u64, 4242]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[7, 0]), Err(Errno::EINVAL));
}

#[test]
fn unprivileged_priority_changes() {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::user(1000, 1000);
    let me = k.pid;
    let pgid = k.procs[&me].info.pgid;
    k.add_process(300, 0, pgid + 1, 1);
    assert_eq!(sys(&mut k, nr::SETPRIORITY, &[PRIO_PROCESS as u64, 0, 4]), Ok(0));
    // Lowering the nice value needs CAP_SYS_NICE; another user's process
    // cannot be changed at all.
    assert_eq!(sys(&mut k, nr::SETPRIORITY, &[PRIO_PROCESS as u64, 0, 2]), Err(Errno::EACCES));
    assert_eq!(sys(&mut k, nr::SETPRIORITY, &[PRIO_PROCESS as u64, 300, 10]), Err(Errno::EPERM));
    assert_eq!(k.thread_state().sched.nice, 4);
}

#[test]
fn scheduling_policies_and_affinity() {
    let mut k = FakeKernel::new();
    let param = DATA;
    k.poke(param, &10i32.to_le_bytes());
    // Without CAP_SYS_NICE and with RLIMIT_RTPRIO 0: no real-time policy.
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[0, SCHED_FIFO as u64, param]), Err(Errno::EPERM));
    *k.creds_mut() = Creds::root();
    k.poke(param, &0i32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SCHED_GETSCHEDULER, &[0]), Ok(SCHED_OTHER as u64));
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[0, SCHED_OTHER as u64, param]), Ok(0));
    k.poke(param, &10i32.to_le_bytes());
    // Real-time policies are recorded for a privileged caller (ADR 0113).
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[0, SCHED_FIFO as u64, param]), Ok(0));
    assert_eq!(sys(&mut k, nr::SCHED_GETSCHEDULER, &[0]), Ok(SCHED_FIFO as u64));
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[0, SCHED_OTHER as u64, param]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_GETPARAM, &[0, param + 8]), Ok(0));
    assert_eq!(k.peek_u32(param + 8), 10);
    k.poke(param, &0i32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[0, SCHED_OTHER as u64, param]), Ok(0));
    assert_eq!(sys(&mut k, nr::SCHED_GETPARAM, &[0, param]), Ok(0));
    assert_eq!(k.peek_u32(param), 0);
    assert_eq!(sys(&mut k, nr::SCHED_GET_PRIORITY_MAX, &[SCHED_FIFO as u64]), Ok(99));
    assert_eq!(sys(&mut k, nr::SCHED_GET_PRIORITY_MIN, &[SCHED_FIFO as u64]), Ok(1));
    assert_eq!(sys(&mut k, nr::SCHED_GET_PRIORITY_MAX, &[SCHED_OTHER as u64]), Ok(0));
    assert_eq!(sys(&mut k, nr::SCHED_GET_PRIORITY_MAX, &[77]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_GETSCHEDULER, &[4242]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::SCHED_GETSCHEDULER, &[(-1i64) as u64]), Err(Errno::EINVAL));
    // One CPU: the mask is CPU 0, in 8 bytes.
    let mask = DATA + 0x100;
    k.poke(mask, &[0xff; 16]);
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[0, 16, mask]), Ok(8));
    assert_eq!(k.peek(mask, 9), [1, 0, 0, 0, 0, 0, 0, 0, 0xff]);
    assert_eq!(sys(&mut k, nr::SCHED_GETAFFINITY, &[0, 4, mask]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_SETAFFINITY, &[0, 8, mask]), Ok(0));
    k.poke(mask, &[2, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(sys(&mut k, nr::SCHED_SETAFFINITY, &[0, 8, mask]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_YIELD, &[]), Ok(0));
}

#[test]
fn scheduling_attributes_are_per_thread() {
    use crate::sys::sched::{SCHED_BATCH, SCHED_IDLE, SCHED_RESET_ON_FORK, SCHED_RR};
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    let me = k.tid;
    let param = DATA;
    k.poke(param, &0i32.to_le_bytes());
    k.add_thread(me + 1);
    let t = (me + 1) as u64;
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[t, SCHED_BATCH as u64, param]), Ok(0));
    assert_eq!(sys(&mut k, nr::SCHED_GETSCHEDULER, &[t]), Ok(SCHED_BATCH as u64));
    assert_eq!(sys(&mut k, nr::SCHED_GETSCHEDULER, &[0]), Ok(SCHED_OTHER as u64));
    assert_eq!(sys(&mut k, nr::SETPRIORITY, &[PRIO_PROCESS as u64, t, 7]), Ok(0));
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_PROCESS as u64, t]), Ok(13));
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_PROCESS as u64, 0]), Ok(20));
    // The group sees every thread.
    assert_eq!(sys(&mut k, nr::SETPRIORITY, &[PRIO_PROCESS as u64, 0, 9]), Ok(0));
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_PGRP as u64, 0]), Ok(13));
    // The flag is reported and kept by sched_setparam.
    let flagged = (SCHED_RR | SCHED_RESET_ON_FORK) as u64;
    k.poke(param, &5i32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[0, flagged, param]), Ok(0));
    k.poke(param, &60i32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SCHED_SETPARAM, &[0, param]), Ok(0));
    assert_eq!(sys(&mut k, nr::SCHED_GETSCHEDULER, &[0]), Ok(flagged));
    let ts = DATA + 0x100;
    assert_eq!(sys(&mut k, nr::SCHED_RR_GET_INTERVAL, &[0, ts]), Ok(0));
    assert_eq!((k.peek_u64(ts), k.peek_u64(ts + 8)), (0, 100_000_000));
    assert_eq!(sys(&mut k, nr::SCHED_RR_GET_INTERVAL, &[t, ts]), Ok(0));
    assert_eq!(k.peek_u64(ts + 8), 0);
    // A new thread starts with its creator's attributes, reset by the flag.
    k.add_thread(me + 2);
    let s = k.me().thread(me + 2).sched;
    assert_eq!((s.policy, s.nice, s.reset_on_fork), (SCHED_OTHER, 0, false));
    // A zombie is still seen with the values it had.
    k.add_process(300, 0, 300, 300);
    k.proc_state(300).thread_mut(300).sched.policy = SCHED_IDLE;
    k.proc_state(300).thread_mut(300).sched.nice = 9;
    k.proc_state(300).remove_thread(300);
    k.procs.get_mut(&300).unwrap().info.zombie = true;
    assert_eq!(sys(&mut k, nr::SCHED_GETSCHEDULER, &[300]), Ok(SCHED_IDLE as u64));
    assert_eq!(sys(&mut k, nr::GETPRIORITY, &[PRIO_PROCESS as u64, 300]), Ok(11));
}

#[test]
fn scheduling_argument_order() {
    let mut k = FakeKernel::new();
    let bad = 1u64;
    let minus1 = (-1i64) as u64;
    // NULL param or a negative pid first, then EFAULT, then ESRCH, then
    // the policy and priority.
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[4242, 77, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[minus1, 0, bad]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[4242, 77, bad]), Err(Errno::EFAULT));
    k.poke(DATA, &0i32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[4242, 77, DATA]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::SCHED_SETSCHEDULER, &[0, 77, DATA]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_SETPARAM, &[minus1, bad]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_GETPARAM, &[4242, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_GETPARAM, &[4242, bad]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::SCHED_RR_GET_INTERVAL, &[minus1, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SCHED_RR_GET_INTERVAL, &[4242, 0]), Err(Errno::ESRCH));
    assert_eq!(sys(&mut k, nr::SCHED_RR_GET_INTERVAL, &[0, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::SCHED_GET_PRIORITY_MIN, &[7]), Ok(0));
}
