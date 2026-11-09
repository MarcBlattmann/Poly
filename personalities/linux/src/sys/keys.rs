//! add_key(2), request_key(2) and keyctl(2): the key retention service
//! (keyrings(7)) on the [`crate::keys`] table.
//!
//! The handlers read their strings and payloads from user memory first,
//! decide under the table's lock, and write results (and the caller's
//! changed keyrings, [`ThreadKeys`]) after it. Poly has no request-key
//! upcall program: constructing a key fails with `ENOENT` and leaves a
//! negative key, and the operations that need an upcall's authority
//! (`KEYCTL_INSTANTIATE` and friends) are `EPERM`. Persistent keyrings,
//! `KEYCTL_GET_SECURITY`, `KEYCTL_SESSION_TO_PARENT` and the crypto
//! operations are `EOPNOTSUPP`.

use crate::args::Args;
use crate::cred::cap;
use crate::errno::{Errno, SysResult};
use crate::kernel::{Clock, LinuxKernel, RandomMode, UserPtr};
use crate::keys::{
    Caller, DESC_MAX, EDEADLK, ENOKEY, Found, KeyTable, KeyType, PAYLOAD_COPY_MAX, ThreadKeys, perm, reqkey,
};
use crate::ns::lock::Mutex;
use crate::signal::deliver::with_own;
use crate::table::{Handler, nr};
use alloc::vec;
use alloc::vec::Vec;

/// The calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[(nr::ADD_KEY, add_key), (nr::REQUEST_KEY, request_key), (nr::KEYCTL, keyctl)];

/// keyctl(2) operations (`linux/keyctl.h`).
pub mod op {
    pub const GET_KEYRING_ID: i32 = 0;
    pub const JOIN_SESSION_KEYRING: i32 = 1;
    pub const UPDATE: i32 = 2;
    pub const REVOKE: i32 = 3;
    pub const CHOWN: i32 = 4;
    pub const SETPERM: i32 = 5;
    pub const DESCRIBE: i32 = 6;
    pub const CLEAR: i32 = 7;
    pub const LINK: i32 = 8;
    pub const UNLINK: i32 = 9;
    pub const SEARCH: i32 = 10;
    pub const READ: i32 = 11;
    pub const INSTANTIATE: i32 = 12;
    pub const NEGATE: i32 = 13;
    pub const SET_REQKEY_KEYRING: i32 = 14;
    pub const SET_TIMEOUT: i32 = 15;
    pub const ASSUME_AUTHORITY: i32 = 16;
    pub const REJECT: i32 = 19;
    pub const INSTANTIATE_IOV: i32 = 20;
    pub const INVALIDATE: i32 = 21;
    pub const MOVE: i32 = 30;
}

/// `KEYCTL_MOVE_EXCL` (`linux/keyctl.h`).
pub const MOVE_EXCL: u32 = 1;

/// The longest key type name, without its NUL.
const TYPE_MAX: usize = 31;
/// The longest callout string, without its NUL.
const CALLOUT_MAX: usize = 4095;

static TABLE: Mutex<KeyTable> = Mutex::new(KeyTable::new());

/// The calling thread as the key rules see it.
fn caller(k: &mut dyn LinuxKernel) -> Caller {
    let tid = k.tid();
    let (creds, keys) = with_own(k, |p| {
        let t = p.thread(tid);
        (t.creds.clone(), t.keys)
    });
    let now = k.clock(Clock::Realtime).map_or(0, |t| t.sec);
    Caller {
        uid: creds.uid,
        fsuid: creds.fsuid,
        fsgid: creds.fsgid,
        groups: creds.groups.to_vec(),
        sys_admin: creds.capable(cap::SYS_ADMIN),
        keys,
        now,
    }
}

/// Store the caller's changed keyrings: its own, and a new process
/// keyring in every thread of the process.
fn commit(k: &mut dyn LinuxKernel, before: ThreadKeys, after: ThreadKeys) {
    let tid = k.tid();
    with_own(k, |p| {
        p.thread_mut(tid).keys = after;
        if after.process != before.process {
            let ids: Vec<i32> = p.thread_ids().collect();
            for id in ids {
                p.thread_mut(id).keys.process = after.process;
            }
        }
    });
}

/// Run `f` on the key table for the caller, then store the caller's
/// keyrings if `f` changed them (also when it failed after making one).
fn with_table<R>(
    k: &mut dyn LinuxKernel,
    f: impl FnOnce(&mut KeyTable, &mut Caller) -> Result<R, Errno>,
) -> Result<R, Errno> {
    let mut c = caller(k);
    let before = c.keys;
    if !TABLE.lock().seeded() {
        let mut b = [0u8; 8];
        let _ = k.random_bytes(&mut b, RandomMode::Insecure);
        let seed = u64::from_le_bytes(b) ^ (c.now as u64).rotate_left(29) ^ u64::from(k.tid() as u32);
        TABLE.lock().seed(seed);
    }
    let result = f(&mut TABLE.lock(), &mut c);
    if c.keys != before {
        commit(k, before, c.keys);
    }
    result
}

/// A NUL-terminated string of at most `max` bytes: `EFAULT` if
/// unreadable (also NULL), `EINVAL` if longer.
fn read_str(k: &mut dyn LinuxKernel, ptr: UserPtr, max: usize) -> Result<Vec<u8>, Errno> {
    let mut buf = vec![0u8; max + 1];
    let len =
        k.strncpy_from_user(ptr, &mut buf).map_err(|e| if e == Errno::ENAMETOOLONG { Errno::EINVAL } else { e })?;
    buf.truncate(len);
    Ok(buf)
}

/// A key type name: `EFAULT` unreadable, `EINVAL` empty, `EPERM` starting
/// with '.' (reserved for the kernel).
fn read_type(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<Vec<u8>, Errno> {
    let ty = read_str(k, ptr, TYPE_MAX)?;
    match ty.first() {
        None => Err(Errno::EINVAL),
        Some(b'.') => Err(Errno::EPERM),
        Some(_) => Ok(ty),
    }
}

/// A payload of `len` bytes: `EINVAL` above 1 MiB (before reading),
/// `EFAULT` if unreadable.
fn read_payload(k: &mut dyn LinuxKernel, ptr: UserPtr, len: u64) -> Result<Vec<u8>, Errno> {
    if len > PAYLOAD_COPY_MAX {
        return Err(Errno::EINVAL);
    }
    let mut payload = vec![0u8; len as usize];
    if len > 0 {
        k.copy_from_user(ptr, &mut payload)?;
    }
    Ok(payload)
}

/// Give `data` to a `buf` of `buflen` bytes if it fits (else write
/// nothing) and return its full length.
fn give(k: &mut dyn LinuxKernel, buf: UserPtr, buflen: u64, data: &[u8]) -> SysResult {
    if !buf.is_null() && buflen >= data.len() as u64 {
        k.copy_to_user(buf, data)?;
    }
    Ok(data.len() as u64)
}

/// add_key(type, description, payload, plen, keyring).
pub fn add_key(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let ty_name = read_type(k, a.ptr(0))?;
    if a.ptr(1).is_null() {
        return Err(Errno::EINVAL);
    }
    let desc = read_str(k, a.ptr(1), DESC_MAX)?;
    if desc.is_empty() {
        return Err(Errno::EINVAL);
    }
    let payload = read_payload(k, a.ptr(2), a.ulong(3))?;
    let ring_id = a.int(4);
    with_table(k, |t, c| {
        let (ring, _) = t.lookup_keyring(c, ring_id, true, perm::WRITE)?;
        let ty = KeyType::from_name(&ty_name).ok_or(Errno::ENODEV)?;
        ty.check_description(&desc)?;
        ty.check_payload(&payload)?;
        t.add(c, ring, ty, &desc, payload).map(u64::from)
    })
}

/// request_key(type, description, callout_info, dest_keyring).
pub fn request_key(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let ty_name = read_type(k, a.ptr(0))?;
    let desc = read_str(k, a.ptr(1), DESC_MAX)?;
    let callout = !a.ptr(2).is_null();
    if callout {
        read_str(k, a.ptr(2), CALLOUT_MAX)?;
    }
    let dest_id = a.int(3);
    with_table(k, |t, c| {
        let dest = match dest_id {
            0 => None,
            id => Some(t.lookup_keyring(c, id, true, perm::WRITE)?.0),
        };
        let ty = KeyType::from_name(&ty_name).ok_or(ENOKEY)?;
        if desc.is_empty() {
            return Err(ENOKEY);
        }
        t.request(c, ty, &desc, callout, dest).map(u64::from)
    })
}

/// keyctl(operation, ...).
pub fn keyctl(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let id = a.int(1);
    match a.int(0) {
        op::GET_KEYRING_ID => get_keyring_id(k, id, a.int(2) != 0),
        op::JOIN_SESSION_KEYRING => join(k, a.ptr(1)),
        op::UPDATE => update(k, id, a.ptr(2), a.ulong(3)),
        op::REVOKE => revoke(k, id),
        op::CHOWN => chown(k, id, a.uint(2), a.uint(3)),
        op::SETPERM => setperm(k, id, a.uint(2)),
        op::DESCRIBE => describe(k, id, a.ptr(2), a.ulong(3)),
        op::CLEAR => clear(k, id),
        op::LINK => link(k, id, a.int(2)),
        op::UNLINK => unlink(k, id, a.int(2)),
        op::SEARCH => search(k, id, a.ptr(2), a.ptr(3), a.int(4)),
        op::READ => read(k, id, a.ptr(2), a.ulong(3)),
        // Only a request-key upcall holds the authority these need.
        op::INSTANTIATE | op::NEGATE | op::REJECT | op::INSTANTIATE_IOV => Err(Errno::EPERM),
        op::SET_REQKEY_KEYRING => set_reqkey_keyring(k, id),
        op::SET_TIMEOUT => set_timeout(k, id, a.uint(2)),
        op::ASSUME_AUTHORITY => match id {
            0 => Ok(0),
            id if id < 0 => Err(Errno::EINVAL),
            // No key is an authorisation key outside an upcall.
            _ => Err(ENOKEY),
        },
        op::INVALIDATE => invalidate(k, id),
        op::MOVE => move_key(k, id, a.int(2), a.int(3), a.uint(4)),
        _ => Err(Errno::EOPNOTSUPP),
    }
}

/// `KEYCTL_GET_KEYRING_ID`: the serial a (special) id names.
fn get_keyring_id(k: &mut dyn LinuxKernel, id: i32, create: bool) -> SysResult {
    with_table(k, |t, c| {
        let (s, _) = t.resolve(c, id, create)?;
        t.get(s).ok_or(ENOKEY)?.check_live(c.now)?;
        Ok(u64::from(s))
    })
}

/// `KEYCTL_JOIN_SESSION_KEYRING`.
fn join(k: &mut dyn LinuxKernel, name: UserPtr) -> SysResult {
    let name = if name.is_null() {
        None
    } else {
        let n = read_str(k, name, DESC_MAX)?;
        match n.first() {
            None => return Err(Errno::EINVAL),
            Some(b'.') => return Err(Errno::EPERM),
            Some(_) => Some(n),
        }
    };
    with_table(k, |t, c| Ok(u64::from(t.join(c, name.as_deref()))))
}

/// `KEYCTL_UPDATE`: a new payload for a user or logon key.
fn update(k: &mut dyn LinuxKernel, id: i32, payload: UserPtr, len: u64) -> SysResult {
    let payload = read_payload(k, payload, len)?;
    with_table(k, |t, c| {
        let (s, _) = t.lookup(c, id, false, perm::WRITE)?;
        let key = t.get_mut(s).ok_or(ENOKEY)?;
        if key.ty == KeyType::Keyring {
            return Err(Errno::EOPNOTSUPP);
        }
        key.ty.check_payload(&payload)?;
        key.payload = payload;
        key.negative = false;
        Ok(0)
    })
}

/// `KEYCTL_REVOKE`: needs `WRITE` or `SETATTR`.
fn revoke(k: &mut dyn LinuxKernel, id: i32) -> SysResult {
    with_table(k, |t, c| {
        let (s, possessed) = t.resolve(c, id, false)?;
        t.get(s).ok_or(ENOKEY)?.check_live(c.now)?;
        if !t.permitted(s, c, possessed, perm::WRITE) && !t.permitted(s, c, possessed, perm::SETATTR) {
            return Err(Errno::EACCES);
        }
        t.get_mut(s).ok_or(ENOKEY)?.revoked = true;
        Ok(0)
    })
}

/// `KEYCTL_CHOWN`: -1 leaves an id alone; a change needs `SETATTR`, and
/// `CAP_SYS_ADMIN` to give the key to another user or to a group the
/// caller is not in.
fn chown(k: &mut dyn LinuxKernel, id: i32, uid: u32, gid: u32) -> SysResult {
    if uid == u32::MAX && gid == u32::MAX {
        return Ok(0);
    }
    with_table(k, |t, c| {
        let (s, _) = t.lookup(c, id, false, perm::SETATTR)?;
        let key = t.get_mut(s).ok_or(ENOKEY)?;
        if uid != u32::MAX && uid != key.uid && !c.sys_admin {
            return Err(Errno::EACCES);
        }
        let member = gid == c.fsgid || c.groups.contains(&gid);
        if gid != u32::MAX && gid != key.gid && !member && !c.sys_admin {
            return Err(Errno::EACCES);
        }
        if uid != u32::MAX {
            key.uid = uid;
        }
        if gid != u32::MAX {
            key.gid = gid;
        }
        Ok(0)
    })
}

/// `KEYCTL_SETPERM`: bits outside the six of each byte are `EINVAL`;
/// needs `SETATTR`.
fn setperm(k: &mut dyn LinuxKernel, id: i32, mask: u32) -> SysResult {
    if mask & !perm::VALID != 0 {
        return Err(Errno::EINVAL);
    }
    with_table(k, |t, c| {
        let (s, _) = t.lookup(c, id, false, perm::SETATTR)?;
        t.get_mut(s).ok_or(ENOKEY)?.perm = mask;
        Ok(0)
    })
}

/// `KEYCTL_DESCRIBE`: "type;uid;gid;perm;description"; needs `VIEW`.
fn describe(k: &mut dyn LinuxKernel, id: i32, buf: UserPtr, buflen: u64) -> SysResult {
    let text = with_table(k, |t, c| {
        let (s, _) = t.lookup(c, id, false, perm::VIEW)?;
        Ok(t.get(s).ok_or(ENOKEY)?.describe())
    })?;
    give(k, buf, buflen, &text)
}

/// `KEYCTL_READ`: needs `READ`, or the key possessed. A logon key's
/// payload never leaves the kernel (`EOPNOTSUPP`).
fn read(k: &mut dyn LinuxKernel, id: i32, buf: UserPtr, buflen: u64) -> SysResult {
    let data = with_table(k, |t, c| {
        let (s, possessed) = t.resolve(c, id, false)?;
        let key = t.get(s).ok_or(ENOKEY)?;
        key.check_live(c.now)?;
        if key.negative {
            return Err(ENOKEY);
        }
        if key.ty == KeyType::Logon {
            return Err(Errno::EOPNOTSUPP);
        }
        if !possessed && !t.permitted(s, c, false, perm::READ) {
            return Err(Errno::EACCES);
        }
        Ok(key.read())
    })?;
    give(k, buf, buflen, &data)
}

/// `KEYCTL_CLEAR`: unlink everything from a keyring; needs `WRITE`.
fn clear(k: &mut dyn LinuxKernel, id: i32) -> SysResult {
    with_table(k, |t, c| {
        let (s, _) = t.lookup_keyring(c, id, false, perm::WRITE)?;
        t.get_mut(s).ok_or(ENOKEY)?.links.clear();
        Ok(0)
    })
}

/// `KEYCTL_LINK`: needs `LINK` on the key and `WRITE` on the keyring;
/// `EDEADLK` for a cycle.
fn link(k: &mut dyn LinuxKernel, key_id: i32, ring_id: i32) -> SysResult {
    with_table(k, |t, c| {
        let (key, _) = t.lookup(c, key_id, false, perm::LINK)?;
        let (ring, _) = t.lookup_keyring(c, ring_id, true, perm::WRITE)?;
        if t.would_cycle(ring, key) {
            return Err(EDEADLK);
        }
        t.link(ring, key);
        Ok(0)
    })
}

/// `KEYCTL_UNLINK`: `ENOENT` if the keyring does not link the key.
fn unlink(k: &mut dyn LinuxKernel, key_id: i32, ring_id: i32) -> SysResult {
    with_table(k, |t, c| {
        let (key, _) = t.resolve(c, key_id, false)?;
        let (ring, _) = t.lookup_keyring(c, ring_id, false, perm::WRITE)?;
        t.unlink(ring, key).map(|()| 0)
    })
}

/// `KEYCTL_SEARCH`: search below a keyring; link the key found into
/// `dest_id` unless it is 0.
fn search(k: &mut dyn LinuxKernel, ring_id: i32, ty: UserPtr, desc: UserPtr, dest_id: i32) -> SysResult {
    let ty_name = read_type(k, ty)?;
    let desc = read_str(k, desc, DESC_MAX)?;
    with_table(k, |t, c| {
        let (ring, possessed) = t.lookup_keyring(c, ring_id, false, perm::SEARCH)?;
        let dest = match dest_id {
            0 => None,
            id => Some(t.lookup_keyring(c, id, true, perm::WRITE)?.0),
        };
        let ty = KeyType::from_name(&ty_name).ok_or(ENOKEY)?;
        if desc.is_empty() {
            return Err(ENOKEY);
        }
        match t.search(c, ring, possessed, ty, &desc, false) {
            Found::Key(s) => {
                if let Some(d) = dest {
                    if !t.permitted(s, c, possessed, perm::LINK) {
                        return Err(Errno::EACCES);
                    }
                    if t.would_cycle(d, s) {
                        return Err(EDEADLK);
                    }
                    t.link(d, s);
                }
                Ok(u64::from(s))
            }
            other => Err(other.error()),
        }
    })
}

/// `KEYCTL_SET_REQKEY_KEYRING`: the previous default; -1 only reports it.
fn set_reqkey_keyring(k: &mut dyn LinuxKernel, which: i32) -> SysResult {
    if which != reqkey::NO_CHANGE && !(0..=reqkey::MAX).contains(&which) {
        return Err(Errno::EINVAL);
    }
    let tid = k.tid();
    Ok(with_own(k, |p| {
        let keys = &mut p.thread_mut(tid).keys;
        let old = keys.reqkey_default;
        if which != reqkey::NO_CHANGE {
            keys.reqkey_default = which as u8;
        }
        u64::from(old)
    }))
}

/// `KEYCTL_SET_TIMEOUT`: expire `seconds` from now (0: never); needs
/// `SETATTR`.
fn set_timeout(k: &mut dyn LinuxKernel, id: i32, seconds: u32) -> SysResult {
    with_table(k, |t, c| {
        let (s, _) = t.lookup(c, id, false, perm::SETATTR)?;
        let at = (seconds != 0).then(|| c.now.saturating_add(i64::from(seconds)));
        t.get_mut(s).ok_or(ENOKEY)?.expiry = at;
        Ok(0)
    })
}

/// `KEYCTL_INVALIDATE`: needs `SEARCH`; the key goes at once.
fn invalidate(k: &mut dyn LinuxKernel, id: i32) -> SysResult {
    with_table(k, |t, c| {
        let (s, possessed) = t.resolve(c, id, false)?;
        if t.get(s).ok_or(ENOKEY)?.expired(c.now) {
            return Err(crate::keys::EKEYEXPIRED);
        }
        if !t.permitted(s, c, possessed, perm::SEARCH) {
            return Err(Errno::EACCES);
        }
        t.invalidate(s);
        Ok(0)
    })
}

/// `KEYCTL_MOVE`: relink a key from one keyring to another.
fn move_key(k: &mut dyn LinuxKernel, key_id: i32, from_id: i32, to_id: i32, flags: u32) -> SysResult {
    if flags & !MOVE_EXCL != 0 {
        return Err(Errno::EINVAL);
    }
    with_table(k, |t, c| {
        let (key, _) = t.lookup(c, key_id, false, perm::LINK)?;
        let (from, _) = t.lookup_keyring(c, from_id, false, perm::WRITE)?;
        let (to, _) = t.lookup_keyring(c, to_id, true, perm::WRITE)?;
        if !t.holds(from, key) {
            return Err(Errno::ENOENT);
        }
        if t.holds(to, key) {
            if flags & MOVE_EXCL != 0 {
                return Err(Errno::EEXIST);
            }
            if from != to {
                t.unlink(from, key)?;
            }
            return Ok(0);
        }
        if t.would_cycle(to, key) {
            return Err(EDEADLK);
        }
        t.link(to, key);
        t.unlink(from, key)?;
        Ok(0)
    })
}
