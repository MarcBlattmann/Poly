//! What `/proc` shows of an IPC namespace (ipc_namespaces(7): "The
//! following /proc interfaces are distinct in each IPC namespace"; ADR
//! 0178):
//!
//! - the listings `/proc/sysvipc/{msg,shm,sem}` (proc_sysvipc(5)): a
//!   header line and one line per object of the namespace;
//! - the System V settings `/proc/sys/kernel/{msgmax,msgmnb,msgmni,
//!   shmmax,shmall,shmmni,sem}` (proc_sys_kernel(5)) and the POSIX message
//!   queue settings `/proc/sys/fs/mqueue/{queues_max,msg_max,
//!   msgsize_max,msg_default,msgsize_default}` (mq_overview(7)): the
//!   limits the namespace's tables carry. A value written in one
//!   namespace is read back there and applies there; every other
//!   namespace keeps its own.
//!
//! Ids in the listings are the kernel's (not translated to the reader's
//! user namespace) and pids are global.

use super::{mqueue, msg, sem, shm};
use crate::errno::Errno;
use crate::ns::IpcNs;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// A file of an IPC namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcFile {
    /// `/proc/sysvipc/msg`, `shm`, `sem`.
    SysvMsg,
    SysvShm,
    SysvSem,
    /// `/proc/sys/kernel/msgmni`, `msgmax`, `msgmnb`.
    Msgmni,
    Msgmax,
    Msgmnb,
    /// `/proc/sys/kernel/shmmax`, `shmall`, `shmmni`.
    Shmmax,
    Shmall,
    Shmmni,
    /// `/proc/sys/kernel/sem`: `SEMMSL`, `SEMMNS`, `SEMOPM`, `SEMMNI`.
    Sem,
    /// `/proc/sys/fs/mqueue/*`.
    MqQueuesMax,
    MqMsgMax,
    MqMsgsizeMax,
    MqMsgDefault,
    MqMsgsizeDefault,
}

/// `IPCMNI` (`linux/ipc.h`): the most objects of a kind a namespace can
/// be set to hold.
pub const IPCMNI: u64 = 32768;
/// The least `msgsize_max` and `msgsize_default` (mq_overview(7):
/// "MQ_MIN_MSGSIZE", 128).
pub const MIN_MSGSIZE: u64 = 128;

fn line(v: u64) -> Vec<u8> {
    format!("{v}\n").into_bytes()
}

/// `/proc/sysvipc/msg`.
fn sysv_msg(t: &msg::MsgTable) -> Vec<u8> {
    let mut out = String::from(
        "       key      msqid perms      cbytes       qnum lspid lrpid   uid   gid  cuid  cgid      stime      rtime      ctime\n",
    );
    for (id, q) in t.iter() {
        out.push_str(&format!(
            "{:>10} {:>10}  {:>4o}  {:>10} {:>10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>10} {:>10} {:>10}\n",
            q.key,
            id,
            q.mode,
            q.cbytes(),
            q.qnum(),
            q.lspid,
            q.lrpid,
            q.uid,
            q.gid,
            q.cuid,
            q.cgid,
            q.stime,
            q.rtime,
            q.ctime
        ));
    }
    out.into_bytes()
}

/// `/proc/sysvipc/shm`. Nothing of a segment is counted as resident or
/// swapped (the last two columns are 0).
fn sysv_shm(t: &shm::ShmTable) -> Vec<u8> {
    let mut out = String::from(
        "       key      shmid perms                  size  cpid  lpid nattch   uid   gid  cuid  cgid      atime      dtime      ctime                   rss                  swap\n",
    );
    for (id, s) in t.iter() {
        out.push_str(&format!(
            "{:>10} {:>10}  {:>4o} {:>21} {:>5} {:>5}  {:>5} {:>5} {:>5} {:>5} {:>5} {:>10} {:>10} {:>10} {:>21} {:>21}\n",
            s.key,
            id,
            s.mode,
            s.segsz,
            s.cpid,
            s.lpid,
            s.nattch(),
            s.uid,
            s.gid,
            s.cuid,
            s.cgid,
            s.atime,
            s.dtime,
            s.ctime,
            0,
            0
        ));
    }
    out.into_bytes()
}

/// `/proc/sysvipc/sem`.
fn sysv_sem(t: &sem::SemTable) -> Vec<u8> {
    let mut out =
        String::from("       key      semid perms      nsems   uid   gid  cuid  cgid      otime      ctime\n");
    for (id, s) in t.iter() {
        out.push_str(&format!(
            "{:>10} {:>10}  {:>4o} {:>10} {:>5} {:>5} {:>5} {:>5} {:>10} {:>10}\n",
            s.key,
            id,
            s.mode,
            s.nsems(),
            s.uid,
            s.gid,
            s.cuid,
            s.cgid,
            s.otime,
            s.ctime
        ));
    }
    out.into_bytes()
}

/// The text of `file` in IPC namespace `ns` (`None`: the initial one).
pub fn read(ns: &Option<Arc<IpcNs>>, file: IpcFile) -> Vec<u8> {
    match file {
        IpcFile::SysvMsg => msg::with_table(ns, |t| sysv_msg(t)),
        IpcFile::SysvShm => shm::with_table(ns, |t| sysv_shm(t)),
        IpcFile::SysvSem => sem::with_table(ns, |t| sysv_sem(t)),
        IpcFile::Msgmni => line(msg::with_table(ns, |t| t.limits.mni) as u64),
        IpcFile::Msgmax => line(msg::with_table(ns, |t| t.limits.max)),
        IpcFile::Msgmnb => line(msg::with_table(ns, |t| t.limits.mnb)),
        IpcFile::Shmmax => line(shm::with_table(ns, |t| t.limits.max)),
        IpcFile::Shmall => line(shm::with_table(ns, |t| t.limits.all)),
        IpcFile::Shmmni => line(shm::with_table(ns, |t| t.limits.mni) as u64),
        IpcFile::Sem => {
            let l = sem::with_table(ns, |t| t.limits);
            format!("{}\t{}\t{}\t{}\n", l.msl, l.mns, l.opm, l.mni).into_bytes()
        }
        IpcFile::MqQueuesMax => line(mqueue::with_names(ns, |t| t.limits.queues_max) as u64),
        IpcFile::MqMsgMax => line(mqueue::with_names(ns, |t| t.limits.msg_max) as u64),
        IpcFile::MqMsgsizeMax => line(mqueue::with_names(ns, |t| t.limits.msgsize_max) as u64),
        IpcFile::MqMsgDefault => line(mqueue::with_names(ns, |t| t.limits.msg_default) as u64),
        IpcFile::MqMsgsizeDefault => line(mqueue::with_names(ns, |t| t.limits.msgsize_default) as u64),
    }
}

/// The decimal numbers in `data`, separated by white space: `EINVAL` for
/// anything else, or for more than `most` of them.
fn numbers(data: &[u8], most: usize) -> Result<Vec<u64>, Errno> {
    let text = core::str::from_utf8(data).map_err(|_| Errno::EINVAL)?;
    let mut out = Vec::new();
    for word in text.split(|c: char| c.is_ascii_whitespace() || c == '\0').filter(|w| !w.is_empty()) {
        if out.len() == most {
            return Err(Errno::EINVAL);
        }
        out.push(word.parse::<u64>().map_err(|_| Errno::EINVAL)?);
    }
    Ok(out)
}

fn within(v: u64, low: u64, high: u64) -> Result<u64, Errno> {
    if (low..=high).contains(&v) { Ok(v) } else { Err(Errno::EINVAL) }
}

/// A write of `data` to the setting `file` of IPC namespace `ns`: the
/// bytes taken (all of them; a write returns the number of bytes written).
/// The value is a decimal number (`sem`: up to four, the first ones of
/// the four; the rest keep their values), white space around it allowed;
/// a write without a number changes nothing. `EINVAL` for anything else
/// and for a value outside the setting's range: `int`s for the System V
/// counts and sizes (at most [`IPCMNI`] objects of a kind), and
/// mq_overview(7)'s floors and ceilings for the message queue settings
/// (`msg_max` and `msg_default` 1 to `HARD_MSGMAX`, the sizes
/// [`MIN_MSGSIZE`] to 16 MiB). The listings cannot be written (`EROFS`).
pub fn write(ns: &Option<Arc<IpcNs>>, file: IpcFile, data: &[u8]) -> Result<usize, Errno> {
    const INT: u64 = i32::MAX as u64;
    if matches!(file, IpcFile::SysvMsg | IpcFile::SysvShm | IpcFile::SysvSem) {
        return Err(Errno::EROFS);
    }
    let values = numbers(data, if file == IpcFile::Sem { 4 } else { 1 })?;
    let Some(&v) = values.first() else { return Ok(data.len()) };
    match file {
        IpcFile::Msgmni => {
            let v = within(v, 0, IPCMNI)? as usize;
            msg::with_table(ns, |t| t.limits.mni = v);
        }
        IpcFile::Msgmax => {
            let v = within(v, 0, INT)?;
            msg::with_table(ns, |t| t.limits.max = v);
        }
        IpcFile::Msgmnb => {
            let v = within(v, 0, INT)?;
            msg::with_table(ns, |t| t.limits.mnb = v);
        }
        IpcFile::Shmmax => shm::with_table(ns, |t| t.limits.max = v),
        IpcFile::Shmall => shm::with_table(ns, |t| t.limits.all = v),
        IpcFile::Shmmni => {
            let v = within(v, 0, IPCMNI)? as usize;
            shm::with_table(ns, |t| t.limits.mni = v);
        }
        IpcFile::Sem => {
            for (i, v) in values.iter().enumerate() {
                within(*v, 0, if i == 3 { IPCMNI } else { INT })?;
            }
            sem::with_table(ns, |t| {
                let l = &mut t.limits;
                for (i, v) in values.iter().enumerate() {
                    match i {
                        0 => l.msl = *v as i32,
                        1 => l.mns = *v,
                        2 => l.opm = *v as u32,
                        _ => l.mni = *v as usize,
                    }
                }
            });
        }
        IpcFile::MqQueuesMax => {
            let v = within(v, 0, INT)? as usize;
            mqueue::with_names(ns, |t| t.limits.queues_max = v);
        }
        IpcFile::MqMsgMax => {
            let v = within(v, 1, mqueue::HARD_MSGMAX as u64)? as i64;
            mqueue::with_names(ns, |t| t.limits.msg_max = v);
        }
        IpcFile::MqMsgsizeMax => {
            let v = within(v, MIN_MSGSIZE, mqueue::HARD_MSGSIZEMAX as u64)? as i64;
            mqueue::with_names(ns, |t| t.limits.msgsize_max = v);
        }
        IpcFile::MqMsgDefault => {
            let v = within(v, 1, mqueue::HARD_MSGMAX as u64)? as i64;
            mqueue::with_names(ns, |t| t.limits.msg_default = v);
        }
        IpcFile::MqMsgsizeDefault => {
            let v = within(v, MIN_MSGSIZE, mqueue::HARD_MSGSIZEMAX as u64)? as i64;
            mqueue::with_names(ns, |t| t.limits.msgsize_default = v);
        }
        IpcFile::SysvMsg | IpcFile::SysvShm | IpcFile::SysvSem => return Err(Errno::EROFS),
    }
    Ok(data.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::shm::Who;
    use alloc::vec;

    fn text(ns: &Option<Arc<IpcNs>>, file: IpcFile) -> String {
        String::from_utf8(read(ns, file)).unwrap()
    }

    fn me() -> Who {
        Who { euid: 1000, egid: 1000, groups: Arc::from([]), ipc_owner: false, sys_admin: false, ipc_lock: false }
    }

    /// The ns_uts_ipc sheet: a new namespace lists only the header line;
    /// each object adds a line; another namespace's objects are not there.
    #[test]
    fn sysvipc_lists_the_objects_of_one_namespace() {
        let outer = Some(IpcNs::create(&None));
        let inner = Some(IpcNs::create(&None));
        for file in [IpcFile::SysvMsg, IpcFile::SysvShm, IpcFile::SysvSem] {
            assert_eq!(text(&outer, file).lines().count(), 1, "{file:?}");
        }
        let flags = shm::IPC_CREAT | shm::IPC_EXCL | 0o600;
        assert_eq!(msg::with_table(&outer, |t| t.get(0x706b_0001, flags, &me(), 7)), Ok(0));
        assert_eq!(msg::with_table(&outer, |t| t.get(0x706b_0002, flags, &me(), 8)), Ok(1));
        assert_eq!(sem::with_table(&outer, |t| t.get(0x706b_0001, 3, flags, &me(), 9)), Ok(0));
        let listing = text(&outer, IpcFile::SysvMsg);
        let lines: Vec<&str> = listing.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("       key      msqid perms      cbytes       qnum lspid lrpid"));
        let fields: Vec<&str> = lines[1].split_whitespace().collect();
        assert_eq!(fields[..5], ["1886060545", "0", "600", "0", "0"]);
        assert_eq!(fields[7..11], ["1000", "1000", "1000", "1000"]);
        assert_eq!(fields.len(), 14);
        assert_eq!(lines[2].split_whitespace().nth(1), Some("1"));
        let listing = text(&outer, IpcFile::SysvSem);
        assert_eq!(listing.lines().count(), 2);
        let fields: Vec<&str> = listing.lines().nth(1).unwrap().split_whitespace().collect();
        assert_eq!(fields[..4], ["1886060545", "0", "600", "3"]);
        assert_eq!(text(&outer, IpcFile::SysvShm).lines().count(), 1);
        assert!(text(&outer, IpcFile::SysvShm).ends_with("swap\n"));
        // The other namespace is still empty, and removing brings a
        // listing back to its header.
        assert_eq!(text(&inner, IpcFile::SysvMsg).lines().count(), 1);
        msg::with_table(&outer, |t| {
            t.remove(0).unwrap();
            t.remove(1).unwrap();
        });
        assert_eq!(text(&outer, IpcFile::SysvMsg).lines().count(), 1);
    }

    /// The sheet: "a value written in the inner namespace is read back
    /// there and the outer one keeps its own".
    #[test]
    fn settings_are_per_namespace() {
        let outer = Some(IpcNs::create(&None));
        let inner = Some(IpcNs::create(&None));
        assert_eq!(text(&outer, IpcFile::Msgmni), "32000\n");
        assert_eq!(text(&outer, IpcFile::Msgmax), "8192\n");
        assert_eq!(text(&outer, IpcFile::Msgmnb), "16384\n");
        assert_eq!(text(&outer, IpcFile::Shmmni), "4096\n");
        assert_eq!(text(&outer, IpcFile::Shmmax), "18446744073692774399\n");
        assert_eq!(text(&outer, IpcFile::Sem), "32000\t1024000000\t500\t32000\n");
        assert_eq!(text(&outer, IpcFile::MqQueuesMax), "256\n");
        assert_eq!(text(&outer, IpcFile::MqMsgMax), "10\n");
        assert_eq!(text(&outer, IpcFile::MqMsgsizeMax), "8192\n");
        assert_eq!(text(&outer, IpcFile::MqMsgDefault), "10\n");
        assert_eq!(text(&outer, IpcFile::MqMsgsizeDefault), "8192\n");

        assert_eq!(write(&inner, IpcFile::Msgmni, b"77"), Ok(2));
        assert_eq!(text(&inner, IpcFile::Msgmni), "77\n");
        assert_eq!(write(&inner, IpcFile::MqMsgMax, b"11\n"), Ok(3));
        assert_eq!(text(&inner, IpcFile::MqMsgMax), "11\n");
        assert_eq!(text(&outer, IpcFile::Msgmni), "32000\n");
        assert_eq!(text(&outer, IpcFile::MqMsgMax), "10\n");
        // The initial namespace has its own too.
        assert_eq!(text(&None, IpcFile::Msgmni), "32000\n");

        // Bad values.
        assert_eq!(write(&inner, IpcFile::Msgmni, b"x"), Err(Errno::EINVAL));
        assert_eq!(write(&inner, IpcFile::Msgmni, b"-1"), Err(Errno::EINVAL));
        assert_eq!(write(&inner, IpcFile::Msgmni, b"32769"), Err(Errno::EINVAL));
        assert_eq!(write(&inner, IpcFile::Msgmni, b"1 2"), Err(Errno::EINVAL));
        assert_eq!(write(&inner, IpcFile::MqMsgMax, b"0"), Err(Errno::EINVAL));
        assert_eq!(write(&inner, IpcFile::MqMsgMax, b"65537"), Err(Errno::EINVAL));
        assert_eq!(write(&inner, IpcFile::MqMsgsizeMax, b"127"), Err(Errno::EINVAL));
        assert_eq!(write(&inner, IpcFile::SysvMsg, b"1"), Err(Errno::EROFS));
        assert_eq!(write(&inner, IpcFile::Msgmni, b"\n"), Ok(1), "nothing to set");
        assert_eq!(text(&inner, IpcFile::Msgmni), "77\n");
        // sem takes up to four numbers.
        assert_eq!(write(&inner, IpcFile::Sem, b"250 32000 32 128\n"), Ok(17));
        assert_eq!(text(&inner, IpcFile::Sem), "250\t32000\t32\t128\n");
        assert_eq!(write(&inner, IpcFile::Sem, b"10"), Ok(2));
        assert_eq!(text(&inner, IpcFile::Sem), "10\t32000\t32\t128\n");
        assert_eq!(write(&inner, IpcFile::Sem, b"1 2 3 4 5"), Err(Errno::EINVAL));
    }

    /// The limits a namespace was given apply in it.
    #[test]
    fn a_namespaces_limits_apply_to_its_tables() {
        let ns = Some(IpcNs::create(&None));
        let flags = shm::IPC_CREAT | 0o600;
        write(&ns, IpcFile::Msgmni, b"1").unwrap();
        write(&ns, IpcFile::Msgmnb, b"100").unwrap();
        assert_eq!(msg::with_table(&ns, |t| t.get(1, flags, &me(), 0)), Ok(0));
        assert_eq!(msg::with_table(&ns, |t| t.get(2, flags, &me(), 0)), Err(Errno::ENOSPC));
        assert_eq!(msg::with_table(&ns, |t| t.queue(0).map(|q| q.qbytes)), Some(100));
        write(&ns, IpcFile::Sem, b"2 100 5 1").unwrap();
        assert_eq!(sem::with_table(&ns, |t| t.get(1, 3, flags, &me(), 0)), Err(Errno::EINVAL));
        assert_eq!(sem::with_table(&ns, |t| t.get(1, 2, flags, &me(), 0)), Ok(0));
        assert_eq!(sem::with_table(&ns, |t| t.get(2, 1, flags, &me(), 0)), Err(Errno::ENOSPC));
        // POSIX queues: the defaults and the ceilings of the namespace.
        write(&ns, IpcFile::MqMsgMax, b"4").unwrap();
        write(&ns, IpcFile::MqMsgsizeDefault, b"256").unwrap();
        write(&ns, IpcFile::MqQueuesMax, b"2").unwrap();
        let q = mqueue::with_names(&ns, |t| t.create(b"a", (0o600, 0, 0), None, false)).unwrap();
        assert_eq!((q.maxmsg, q.msgsize), (4, 256), "msg_default is capped by msg_max");
        let big = mqueue::MqAttr { maxmsg: 5, msgsize: 100, ..mqueue::MqAttr::default() };
        assert_eq!(
            mqueue::with_names(&ns, |t| t.create(b"b", (0o600, 0, 0), Some(big), false)).err(),
            Some(Errno::EINVAL)
        );
        assert!(mqueue::with_names(&ns, |t| t.create(b"b", (0o600, 0, 0), Some(big), true)).is_ok());
        assert_eq!(mqueue::with_names(&ns, |t| t.create(b"c", (0o600, 0, 0), None, false)).err(), Some(Errno::ENOSPC));
        // A file system listing: names, ids and removal.
        let entries = mqueue::with_names(&ns, |t| t.entries());
        assert_eq!(entries.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(), vec![b"a".to_vec(), b"b".to_vec()]);
        assert!(mqueue::with_names(&ns, |t| t.by_id(q.id)).is_some());
        assert_eq!(mqueue::with_names(&ns, |t| t.remove(b"a")), Ok(()));
        assert_eq!(mqueue::with_names(&ns, |t| t.remove(b"a")), Err(Errno::ENOENT));
        assert!(mqueue::with_names(&ns, |t| t.by_id(q.id)).is_none());
        // Another namespace has none of this.
        let other = Some(IpcNs::create(&None));
        assert!(mqueue::with_names(&other, |t| t.entries()).is_empty());
        assert_eq!(text(&other, IpcFile::MqMsgMax), "10\n");
    }
}
