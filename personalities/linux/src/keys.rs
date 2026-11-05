//! The kernel key retention service (keyrings(7)): keys, keyrings, their
//! permissions and the searches add_key(2), request_key(2) and keyctl(2)
//! do. The handlers are [`crate::sys::keys`]'s.
//!
//! Every key lives in one [`KeyTable`] under a random positive 31-bit
//! serial. A keyring's payload is the list of keys linked into it; a
//! keyring holds at most one key of a given type and description (linking
//! another displaces it). A thread's own keyrings (thread, process and
//! session keyring, and the request-key default) are serials kept in its
//! [`ThreadKeys`]; the per-user keyrings are made on first use.
//!
//! Possession: a key is possessed when it can be found by searching from
//! the caller's thread, process and session keyrings (the user-session
//! keyring when no session was joined), which needs `SEARCH` permission
//! on the key and on the keyrings on the way; keyrings named by a special
//! id are possessed. The possessor byte of a possessed key's permissions
//! adds to the user, group or other byte that applies.
//!
//! Keys are removed from the table only when invalidated (unlinked from
//! every keyring at once); a revoked or expired key stays.

use crate::errno::Errno;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;

const fn errno(n: u16) -> Errno {
    match Errno::new(n) {
        Some(e) => e,
        None => panic!("error numbers are 1..4095"),
    }
}

/// `EDEADLK` (`asm-generic/errno.h`): a link would make a keyring cycle.
pub const EDEADLK: Errno = errno(35);
/// `ENOKEY` (`asm-generic/errno.h`): "Required key not available".
pub const ENOKEY: Errno = errno(126);
/// `EKEYEXPIRED` (`asm-generic/errno.h`): "Key has expired".
pub const EKEYEXPIRED: Errno = errno(127);
/// `EKEYREVOKED` (`asm-generic/errno.h`): "Key has been revoked".
pub const EKEYREVOKED: Errno = errno(128);

/// Special keyring ids (`linux/keyctl.h`).
pub mod spec {
    pub const THREAD_KEYRING: i32 = -1;
    pub const PROCESS_KEYRING: i32 = -2;
    pub const SESSION_KEYRING: i32 = -3;
    pub const USER_KEYRING: i32 = -4;
    pub const USER_SESSION_KEYRING: i32 = -5;
    pub const GROUP_KEYRING: i32 = -6;
    pub const REQKEY_AUTH_KEY: i32 = -7;
    pub const REQUESTOR_KEYRING: i32 = -8;
}

/// Request-key default keyrings (`linux/keyctl.h` `KEY_REQKEY_DEFL_*`).
pub mod reqkey {
    pub const NO_CHANGE: i32 = -1;
    pub const DEFAULT: u8 = 0;
    pub const THREAD_KEYRING: u8 = 1;
    pub const PROCESS_KEYRING: u8 = 2;
    pub const SESSION_KEYRING: u8 = 3;
    pub const USER_KEYRING: u8 = 4;
    pub const USER_SESSION_KEYRING: u8 = 5;
    /// The highest value (`KEY_REQKEY_DEFL_REQUESTOR_KEYRING`).
    pub const MAX: i32 = 7;
}

/// Permission bits of one byte of a key's mask (keyctl_setperm(3),
/// `keyutils.h`: `KEY_*_VIEW` .. `KEY_*_SETATTR`). The possessor byte is
/// bits 24-31, user 16-23, group 8-15, other 0-7.
pub mod perm {
    pub const VIEW: u32 = 0x01;
    pub const READ: u32 = 0x02;
    pub const WRITE: u32 = 0x04;
    pub const SEARCH: u32 = 0x08;
    pub const LINK: u32 = 0x10;
    pub const SETATTR: u32 = 0x20;
    /// Every bit a mask may have.
    pub const VALID: u32 = 0x3f3f_3f3f;
}

/// A new user or keyring key's mask: possessor all, user view.
pub const PERM_KEY: u32 = 0x3f01_0000;
/// A new logon key's mask: possessor all but read, user view.
pub const PERM_LOGON: u32 = 0x3d01_0000;
/// An anonymous session keyring's mask.
pub const PERM_SESSION: u32 = 0x3f03_0000;
/// A named session keyring's mask.
pub const PERM_NAMED_SESSION: u32 = 0x3f13_0000;

/// The longest description, without its NUL.
pub const DESC_MAX: usize = 4095;
/// The largest user or logon payload.
pub const USER_PAYLOAD_MAX: usize = 32767;
/// The largest payload add_key(2) and `KEYCTL_UPDATE` copy at all.
pub const PAYLOAD_COPY_MAX: u64 = 1 << 20;
/// How long a negative key from a failed construction lasts, in seconds.
pub const NEGATIVE_TIMEOUT: i64 = 60;

/// The key types Poly has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyType {
    User,
    Logon,
    Keyring,
}

impl KeyType {
    pub fn from_name(name: &[u8]) -> Option<KeyType> {
        match name {
            b"user" => Some(KeyType::User),
            b"logon" => Some(KeyType::Logon),
            b"keyring" => Some(KeyType::Keyring),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            KeyType::User => "user",
            KeyType::Logon => "logon",
            KeyType::Keyring => "keyring",
        }
    }

    /// The mask a new key of the type gets.
    pub const fn default_perm(self) -> u32 {
        match self {
            KeyType::Logon => PERM_LOGON,
            _ => PERM_KEY,
        }
    }

    /// Check a payload for a new key or an update: user and logon keys
    /// take 1..=32767 bytes, keyrings none.
    pub fn check_payload(self, payload: &[u8]) -> Result<(), Errno> {
        let ok = match self {
            KeyType::User | KeyType::Logon => !payload.is_empty() && payload.len() <= USER_PAYLOAD_MAX,
            KeyType::Keyring => payload.is_empty(),
        };
        if ok { Ok(()) } else { Err(Errno::EINVAL) }
    }

    /// Check a description for a new key: a logon key's is
    /// `<prefix>:<rest>` with a non-empty prefix; a keyring's must not
    /// start with '.'.
    pub fn check_description(self, desc: &[u8]) -> Result<(), Errno> {
        match self {
            KeyType::Logon if !matches!(desc.iter().position(|&b| b == b':'), Some(i) if i > 0) => Err(Errno::EINVAL),
            KeyType::Keyring if desc.first() == Some(&b'.') => Err(Errno::EPERM),
            _ => Ok(()),
        }
    }
}

/// A thread's keyrings (serials, 0 for none) and its request-key default.
/// A new thread starts without a thread keyring; a fork(2) child also
/// without a process keyring; the session keyring and the default are
/// inherited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadKeys {
    pub thread: u32,
    pub process: u32,
    pub session: u32,
    pub reqkey_default: u8,
}

impl ThreadKeys {
    /// What a new thread of the same process starts with.
    pub const fn for_thread(&self) -> ThreadKeys {
        ThreadKeys { thread: 0, ..*self }
    }

    /// What the thread of a fork(2) child starts with.
    pub const fn for_fork(&self) -> ThreadKeys {
        ThreadKeys { thread: 0, process: 0, ..*self }
    }
}

/// One key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub ty: KeyType,
    pub desc: Vec<u8>,
    pub uid: u32,
    pub gid: u32,
    pub perm: u32,
    /// A user or logon key's payload.
    pub payload: Vec<u8>,
    /// A keyring's keys, in link order.
    pub links: Vec<u32>,
    pub revoked: bool,
    /// Negatively instantiated by a failed construction: found, but
    /// unusable (`ENOKEY`).
    pub negative: bool,
    /// The wall-clock second from which the key is expired.
    pub expiry: Option<i64>,
}

impl Key {
    pub fn expired(&self, now: i64) -> bool {
        self.expiry.is_some_and(|at| now >= at)
    }

    /// `EKEYREVOKED`, `EKEYEXPIRED` or fine.
    pub fn check_live(&self, now: i64) -> Result<(), Errno> {
        if self.revoked {
            Err(EKEYREVOKED)
        } else if self.expired(now) {
            Err(EKEYEXPIRED)
        } else {
            Ok(())
        }
    }

    /// The `KEYCTL_DESCRIBE` text with its NUL.
    pub fn describe(&self) -> Vec<u8> {
        use core::fmt::Write;
        let mut s = alloc::string::String::new();
        let _ = write!(s, "{};{};{};{:08x};", self.ty.name(), self.uid, self.gid, self.perm);
        let mut out = s.into_bytes();
        out.extend_from_slice(&self.desc);
        out.push(0);
        out
    }

    /// The `KEYCTL_READ` data: the payload, or a keyring's serials as
    /// 32-bit numbers.
    pub fn read(&self) -> Vec<u8> {
        match self.ty {
            KeyType::Keyring => self.links.iter().flat_map(|s| s.to_le_bytes()).collect(),
            _ => self.payload.clone(),
        }
    }
}

/// Who is calling: the ids permissions are checked against and the
/// thread's keyrings (updated when a call makes a thread or process
/// keyring or joins a session).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caller {
    /// The real user id, which names the user keyrings.
    pub uid: u32,
    pub fsuid: u32,
    pub fsgid: u32,
    pub groups: Vec<u32>,
    /// `CAP_SYS_ADMIN`: `KEYCTL_CHOWN` to another user.
    pub sys_admin: bool,
    pub keys: ThreadKeys,
    /// The wall-clock second.
    pub now: i64,
}

/// What a search found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    Key(u32),
    /// A negative key: `ENOKEY`.
    Negative,
    Revoked,
    Expired,
    /// A match the caller may not search.
    Denied,
    Nothing,
}

impl Found {
    /// The error for anything but a key.
    pub fn error(self) -> Errno {
        match self {
            Found::Revoked => EKEYREVOKED,
            Found::Expired => EKEYEXPIRED,
            Found::Denied => Errno::EACCES,
            _ => ENOKEY,
        }
    }
}

/// Every key.
#[derive(Debug, Default)]
pub struct KeyTable {
    keys: BTreeMap<u32, Key>,
    /// Each user's (user keyring, user-session keyring).
    users: BTreeMap<u32, (u32, u32)>,
    /// The serial generator's state (0: not seeded yet).
    state: u64,
}

impl KeyTable {
    pub const fn new() -> KeyTable {
        KeyTable { keys: BTreeMap::new(), users: BTreeMap::new(), state: 0 }
    }

    pub fn seeded(&self) -> bool {
        self.state != 0
    }

    /// Seed the serial generator once.
    pub fn seed(&mut self, seed: u64) {
        if self.state == 0 {
            self.state = seed | 1;
        }
    }

    /// A random unused serial (positive, 31 bits).
    fn next_serial(&mut self) -> u32 {
        if self.state == 0 {
            self.state = 0x2545_f491_4f6c_dd1d;
        }
        loop {
            // xorshift64*
            let mut x = self.state;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.state = x;
            let v = (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as u32 & 0x7fff_ffff;
            if v > 3 && !self.keys.contains_key(&v) {
                return v;
            }
        }
    }

    pub fn get(&self, serial: u32) -> Option<&Key> {
        self.keys.get(&serial)
    }

    pub fn get_mut(&mut self, serial: u32) -> Option<&mut Key> {
        self.keys.get_mut(&serial)
    }

    fn exists(&self, serial: u32) -> bool {
        serial != 0 && self.keys.contains_key(&serial)
    }

    /// Make a key and return its serial.
    pub fn create(&mut self, ty: KeyType, desc: &[u8], payload: Vec<u8>, uid: u32, gid: u32, perm: u32) -> u32 {
        let serial = self.next_serial();
        let key = Key {
            ty,
            desc: desc.to_vec(),
            uid,
            gid,
            perm,
            payload,
            links: Vec::new(),
            revoked: false,
            negative: false,
            expiry: None,
        };
        self.keys.insert(serial, key);
        serial
    }

    fn create_keyring(&mut self, desc: &[u8], c: &Caller, perm: u32) -> u32 {
        self.create(KeyType::Keyring, desc, Vec::new(), c.fsuid, c.fsgid, perm)
    }

    /// The caller's user keyring and user-session keyring, made on first
    /// use ("_uid.<uid>", "_uid_ses.<uid>", user-keyring(7)); the
    /// user-session keyring links the user keyring.
    pub fn user_keyrings(&mut self, c: &Caller) -> (u32, u32) {
        if let Some(&(u, us)) = self.users.get(&c.uid)
            && self.exists(u)
            && self.exists(us)
        {
            return (u, us);
        }
        let name = |prefix: &str| {
            let mut v = Vec::from(prefix.as_bytes());
            v.extend_from_slice(alloc::format!("{}", c.uid).as_bytes());
            v
        };
        let u = self.create(KeyType::Keyring, &name("_uid."), Vec::new(), c.uid, c.fsgid, PERM_KEY);
        let us = self.create(KeyType::Keyring, &name("_uid_ses."), Vec::new(), c.uid, c.fsgid, PERM_KEY);
        self.link(us, u);
        self.users.insert(c.uid, (u, us));
        (u, us)
    }

    /// The session keyring searches start from: the joined one, else the
    /// user-session keyring.
    pub fn session(&mut self, c: &Caller) -> u32 {
        if self.exists(c.keys.session) { c.keys.session } else { self.user_keyrings(c).1 }
    }

    /// The thread, process and session keyrings (0 for those missing).
    fn roots(&mut self, c: &Caller) -> [u32; 3] {
        let session = self.session(c);
        let have = |s: u32| if self.exists(s) { s } else { 0 };
        [have(c.keys.thread), have(c.keys.process), session]
    }

    /// The permission bits of `key` that apply to `c`.
    fn mask(key: &Key, c: &Caller, possessed: bool) -> u32 {
        let byte = if key.uid == c.fsuid {
            key.perm >> 16
        } else if key.gid == c.fsgid || c.groups.contains(&key.gid) {
            key.perm >> 8
        } else {
            key.perm
        };
        let mut mask = byte & 0x3f;
        if possessed {
            mask |= (key.perm >> 24) & 0x3f;
        }
        mask
    }

    /// Whether `c` has every bit of `need` on key `serial`.
    pub fn permitted(&self, serial: u32, c: &Caller, possessed: bool, need: u32) -> bool {
        self.keys.get(&serial).is_some_and(|k| Self::mask(k, c, possessed) & need == need)
    }

    /// Whether a search may go into keyring `k`.
    fn searchable_ring(k: &Key, c: &Caller, possessed: bool) -> bool {
        k.ty == KeyType::Keyring
            && !k.revoked
            && !k.negative
            && !k.expired(c.now)
            && Self::mask(k, c, possessed) & perm::SEARCH != 0
    }

    /// Every key `c` possesses.
    pub fn possessed_set(&mut self, c: &Caller) -> BTreeSet<u32> {
        let roots = self.roots(c);
        let mut set = BTreeSet::new();
        let mut queue = VecDeque::new();
        for r in roots.into_iter().filter(|&r| r != 0) {
            if set.insert(r) {
                queue.push_back(r);
            }
        }
        while let Some(r) = queue.pop_front() {
            let Some(ring) = self.keys.get(&r) else { continue };
            if !Self::searchable_ring(ring, c, true) {
                continue;
            }
            for &l in &ring.links {
                let Some(key) = self.keys.get(&l) else { continue };
                if Self::mask(key, c, true) & perm::SEARCH != 0 && set.insert(l) && key.ty == KeyType::Keyring {
                    queue.push_back(l);
                }
            }
        }
        set
    }

    /// Whether `c` possesses key `serial`.
    pub fn possessed(&mut self, c: &Caller, serial: u32) -> bool {
        self.possessed_set(c).contains(&serial)
    }

    /// The key a key id names, and whether it is possessed. Special ids
    /// name the caller's keyrings, which are possessed; the thread and
    /// process keyrings are made if `create` (else `ENOKEY` if missing).
    pub fn resolve(&mut self, c: &mut Caller, id: i32, create: bool) -> Result<(u32, bool), Errno> {
        match id {
            0 => Err(Errno::EINVAL),
            spec::THREAD_KEYRING | spec::PROCESS_KEYRING => {
                let thread = id == spec::THREAD_KEYRING;
                let have = if thread { c.keys.thread } else { c.keys.process };
                if self.exists(have) {
                    return Ok((have, true));
                }
                if !create {
                    return Err(ENOKEY);
                }
                let s = self.create_keyring(if thread { b"_tid" } else { b"_pid" }, c, PERM_KEY);
                if thread {
                    c.keys.thread = s;
                } else {
                    c.keys.process = s;
                }
                Ok((s, true))
            }
            spec::SESSION_KEYRING => Ok((self.session(c), true)),
            spec::USER_KEYRING => Ok((self.user_keyrings(c).0, true)),
            spec::USER_SESSION_KEYRING => Ok((self.user_keyrings(c).1, true)),
            spec::REQKEY_AUTH_KEY | spec::REQUESTOR_KEYRING => Err(ENOKEY),
            id if id < 0 => Err(Errno::EINVAL),
            id => {
                let s = id as u32;
                if !self.exists(s) {
                    return Err(ENOKEY);
                }
                Ok((s, self.possessed(c, s)))
            }
        }
    }

    /// [`resolve`](Self::resolve), then `EKEYREVOKED` / `EKEYEXPIRED`
    /// and `EACCES` without every bit of `need`.
    pub fn lookup(&mut self, c: &mut Caller, id: i32, create: bool, need: u32) -> Result<(u32, bool), Errno> {
        let (s, possessed) = self.resolve(c, id, create)?;
        self.keys[&s].check_live(c.now)?;
        if !self.permitted(s, c, possessed, need) {
            return Err(Errno::EACCES);
        }
        Ok((s, possessed))
    }

    /// [`lookup`](Self::lookup) of a keyring: `ENOTDIR` for another key.
    pub fn lookup_keyring(&mut self, c: &mut Caller, id: i32, create: bool, need: u32) -> Result<(u32, bool), Errno> {
        let (s, possessed) = self.resolve(c, id, create)?;
        let key = &self.keys[&s];
        if key.ty != KeyType::Keyring {
            return Err(Errno::ENOTDIR);
        }
        key.check_live(c.now)?;
        if !self.permitted(s, c, possessed, need) {
            return Err(Errno::EACCES);
        }
        Ok((s, possessed))
    }

    /// The key in keyring `ring` of type `ty` and description `desc`.
    fn find_in(&self, ring: u32, ty: KeyType, desc: &[u8]) -> Option<u32> {
        let ring = self.keys.get(&ring)?;
        ring.links.iter().copied().find(|l| self.keys.get(l).is_some_and(|k| k.ty == ty && k.desc == desc))
    }

    /// Link `key` into keyring `ring`, displacing a key of the same type
    /// and description; linking again changes nothing.
    pub fn link(&mut self, ring: u32, key: u32) {
        let Some(k) = self.keys.get(&key) else { return };
        let (ty, desc) = (k.ty, k.desc.clone());
        let displaced: Vec<u32> = match self.keys.get(&ring) {
            Some(r) => r
                .links
                .iter()
                .copied()
                .filter(|&l| l != key && self.keys.get(&l).is_some_and(|o| o.ty == ty && o.desc == desc))
                .collect(),
            None => return,
        };
        let Some(r) = self.keys.get_mut(&ring) else { return };
        r.links.retain(|l| !displaced.contains(l));
        if !r.links.contains(&key) {
            r.links.push(key);
        }
    }

    /// Remove the link to `key` from `ring`: `ENOENT` if there is none.
    pub fn unlink(&mut self, ring: u32, key: u32) -> Result<(), Errno> {
        let r = self.keys.get_mut(&ring).ok_or(ENOKEY)?;
        let before = r.links.len();
        r.links.retain(|&l| l != key);
        if r.links.len() == before { Err(Errno::ENOENT) } else { Ok(()) }
    }

    /// Whether `ring` links `key` directly.
    pub fn holds(&self, ring: u32, key: u32) -> bool {
        self.keys.get(&ring).is_some_and(|r| r.links.contains(&key))
    }

    /// Whether linking `key` into `ring` would make a cycle: `key` is
    /// `ring` or a keyring `ring` can be reached from.
    pub fn would_cycle(&self, ring: u32, key: u32) -> bool {
        if self.keys.get(&key).is_none_or(|k| k.ty != KeyType::Keyring) {
            return false;
        }
        let mut seen = BTreeSet::new();
        let mut stack = alloc::vec![key];
        while let Some(s) = stack.pop() {
            if s == ring {
                return true;
            }
            if !seen.insert(s) {
                continue;
            }
            if let Some(k) = self.keys.get(&s) {
                stack.extend(
                    k.links.iter().copied().filter(|l| self.keys.get(l).is_some_and(|x| x.ty == KeyType::Keyring)),
                );
            }
        }
        false
    }

    /// Search keyring `start` and the keyrings below it, breadth first,
    /// for a key of type `ty` and description `desc`. A usable match wins;
    /// otherwise the first unusable one decides (`skip_expired`: expired
    /// keys are passed over as if absent).
    pub fn search(
        &self,
        c: &Caller,
        start: u32,
        possessed: bool,
        ty: KeyType,
        desc: &[u8],
        skip_expired: bool,
    ) -> Found {
        let mut first = Found::Nothing;
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        seen.insert(start);
        while let Some(r) = queue.pop_front() {
            let Some(ring) = self.keys.get(&r) else { continue };
            if !Self::searchable_ring(ring, c, possessed) {
                continue;
            }
            for &l in &ring.links {
                let Some(key) = self.keys.get(&l) else { continue };
                if key.ty != ty || key.desc != desc {
                    continue;
                }
                let outcome = if key.revoked {
                    Found::Revoked
                } else if key.expired(c.now) {
                    if skip_expired {
                        continue;
                    }
                    Found::Expired
                } else if Self::mask(key, c, possessed) & perm::SEARCH == 0 {
                    Found::Denied
                } else if key.negative {
                    return Found::Negative;
                } else {
                    return Found::Key(l);
                };
                if first == Found::Nothing {
                    first = outcome;
                }
            }
            for &l in &ring.links {
                if self.keys.get(&l).is_some_and(|k| k.ty == KeyType::Keyring) && seen.insert(l) {
                    queue.push_back(l);
                }
            }
        }
        first
    }

    /// Unlink `serial` from every keyring and forget it.
    pub fn invalidate(&mut self, serial: u32) {
        self.keys.remove(&serial);
        for k in self.keys.values_mut() {
            k.links.retain(|&l| l != serial);
        }
        self.users.retain(|_, (u, us)| *u != serial && *us != serial);
    }

    /// The keyring a constructed key goes to when request_key(2) was
    /// given none: the request-key default (keyctl(2)
    /// `KEYCTL_SET_REQKEY_KEYRING`); `DEFAULT` takes the thread keyring,
    /// else the process keyring, else the session keyring, else the
    /// user-session keyring.
    pub fn default_dest(&mut self, c: &mut Caller) -> u32 {
        let pick = match c.keys.reqkey_default {
            reqkey::THREAD_KEYRING => self.resolve(c, spec::THREAD_KEYRING, true).ok(),
            reqkey::PROCESS_KEYRING => self.resolve(c, spec::PROCESS_KEYRING, true).ok(),
            reqkey::SESSION_KEYRING => Some((self.session(c), true)),
            reqkey::USER_KEYRING => Some((self.user_keyrings(c).0, true)),
            reqkey::USER_SESSION_KEYRING => Some((self.user_keyrings(c).1, true)),
            _ => None,
        };
        if let Some((s, _)) = pick {
            return s;
        }
        let [thread, process, session] = self.roots(c);
        [thread, process].into_iter().find(|&s| s != 0).unwrap_or(session)
    }

    /// request_key(2) once its arguments are read: search the thread,
    /// process and session keyrings in that order. A hit is linked into
    /// `dest` if given. A miss with `callout` constructs the key, which
    /// fails (no /sbin/request-key) and leaves a negative key in the
    /// destination: `ENOENT`.
    pub fn request(
        &mut self,
        c: &mut Caller,
        ty: KeyType,
        desc: &[u8],
        callout: bool,
        dest: Option<u32>,
    ) -> Result<u32, Errno> {
        let roots = self.roots(c);
        let mut first: Option<Found> = None;
        for root in roots.into_iter().filter(|&r| r != 0) {
            match self.search(c, root, true, ty, desc, true) {
                Found::Key(s) => {
                    if let Some(d) = dest
                        && !self.would_cycle(d, s)
                    {
                        self.link(d, s);
                    }
                    return Ok(s);
                }
                Found::Negative => return Err(ENOKEY),
                other => {
                    first.get_or_insert(other);
                }
            }
        }
        let err = first.unwrap_or(Found::Nothing).error();
        if err != ENOKEY || !callout {
            return Err(err);
        }
        if ty == KeyType::Keyring {
            return Err(Errno::EPERM);
        }
        let dest = match dest {
            Some(d) => d,
            None => self.default_dest(c),
        };
        let s = self.create(ty, desc, Vec::new(), c.fsuid, c.fsgid, ty.default_perm());
        if let Some(k) = self.keys.get_mut(&s) {
            k.negative = true;
            k.expiry = Some(c.now.saturating_add(NEGATIVE_TIMEOUT));
        }
        self.link(dest, s);
        Err(Errno::ENOENT)
    }

    /// `KEYCTL_JOIN_SESSION_KEYRING`: a new anonymous session keyring
    /// (`None`), or the keyring called `name` the caller owns or may
    /// search, made if there is none; it becomes the session keyring.
    pub fn join(&mut self, c: &mut Caller, name: Option<&[u8]>) -> u32 {
        let s = match name {
            None => self.create_keyring(b"_ses", c, PERM_SESSION),
            Some(name) => {
                let found = self.keys.iter().find(|&(_, k)| {
                    k.ty == KeyType::Keyring
                        && k.desc == name
                        && !k.revoked
                        && !k.negative
                        && !k.expired(c.now)
                        && (k.uid == c.fsuid || Self::mask(k, c, false) & perm::SEARCH != 0)
                });
                match found {
                    Some((&s, _)) => s,
                    None => self.create_keyring(name, c, PERM_NAMED_SESSION),
                }
            }
        };
        c.keys.session = s;
        s
    }

    /// add_key(2) once its arguments are read and the destination keyring
    /// `ring` is checked: update the user or logon key of that type and
    /// description in `ring` (needs `WRITE`), or make a new key and link
    /// it, displacing a keyring of that description.
    pub fn add(&mut self, c: &Caller, ring: u32, ty: KeyType, desc: &[u8], payload: Vec<u8>) -> Result<u32, Errno> {
        if ty != KeyType::Keyring
            && let Some(old) = self.find_in(ring, ty, desc)
        {
            let key = &self.keys[&old];
            if !key.revoked && !key.negative && !key.expired(c.now) {
                // Reached through the destination keyring the caller may
                // write to; possessed if that keyring is.
                let possessed = self.possessed(c, old);
                if !self.permitted(old, c, possessed, perm::WRITE) {
                    return Err(Errno::EACCES);
                }
                if let Some(k) = self.keys.get_mut(&old) {
                    k.payload = payload;
                }
                return Ok(old);
            }
        }
        let s = self.create(ty, desc, payload, c.fsuid, c.fsgid, ty.default_perm());
        self.link(ring, s);
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller() -> Caller {
        Caller { uid: 1000, fsuid: 1000, fsgid: 1000, now: 100, ..Caller::default() }
    }

    fn session(t: &mut KeyTable, c: &mut Caller) -> u32 {
        t.join(c, None)
    }

    #[test]
    fn serials_are_positive_and_unique() {
        let mut t = KeyTable::new();
        t.seed(42);
        let mut seen = BTreeSet::new();
        for _ in 0..1000 {
            let s = t.create(KeyType::User, b"k", alloc::vec![1], 0, 0, PERM_KEY);
            assert!(s > 0 && s <= 0x7fff_ffff);
            assert!(seen.insert(s));
        }
    }

    #[test]
    fn describe_text() {
        let mut t = KeyTable::new();
        let s = t.create(KeyType::User, b"f_k1", alloc::vec![1], 0, 0, PERM_KEY);
        assert_eq!(t.get(s).unwrap().describe(), b"user;0;0;3f010000;f_k1\0");
        let mut c = caller();
        let ses = session(&mut t, &mut c);
        assert_eq!(t.get(ses).unwrap().describe(), b"keyring;1000;1000;3f030000;_ses\0");
    }

    #[test]
    fn add_updates_same_description() {
        let mut t = KeyTable::new();
        let mut c = caller();
        let s = session(&mut t, &mut c);
        let a = t.add(&c, s, KeyType::User, b"k", b"one".to_vec()).unwrap();
        let b = t.add(&c, s, KeyType::User, b"k", b"two".to_vec()).unwrap();
        assert_eq!(a, b);
        assert_eq!(t.get(a).unwrap().payload, b"two");
        let r1 = t.add(&c, s, KeyType::Keyring, b"r", Vec::new()).unwrap();
        let r2 = t.add(&c, s, KeyType::Keyring, b"r", Vec::new()).unwrap();
        assert_ne!(r1, r2, "keyrings are replaced, not updated");
        assert_eq!(t.get(s).unwrap().links, [a, r2]);
    }

    #[test]
    fn special_ids() {
        let mut t = KeyTable::new();
        let mut c = caller();
        assert_eq!(t.resolve(&mut c, spec::THREAD_KEYRING, false), Err(ENOKEY));
        let (th, p) = t.resolve(&mut c, spec::THREAD_KEYRING, true).unwrap();
        assert!(p);
        assert_eq!(c.keys.thread, th);
        assert_eq!(t.resolve(&mut c, spec::THREAD_KEYRING, false), Ok((th, true)));
        assert_eq!(t.resolve(&mut c, spec::GROUP_KEYRING, true), Err(Errno::EINVAL));
        assert_eq!(t.resolve(&mut c, spec::REQKEY_AUTH_KEY, true), Err(ENOKEY));
        assert_eq!(t.resolve(&mut c, spec::REQUESTOR_KEYRING, true), Err(ENOKEY));
        assert_eq!(t.resolve(&mut c, -9, true), Err(Errno::EINVAL));
        assert_eq!(t.resolve(&mut c, 0, true), Err(Errno::EINVAL));
        let (u, _) = t.resolve(&mut c, spec::USER_KEYRING, false).unwrap();
        let (us, _) = t.resolve(&mut c, spec::USER_SESSION_KEYRING, false).unwrap();
        assert_ne!(u, us);
        assert_eq!(t.resolve(&mut c, spec::SESSION_KEYRING, false), Ok((us, true)), "no session joined");
        let s = session(&mut t, &mut c);
        assert_eq!(t.resolve(&mut c, spec::SESSION_KEYRING, false), Ok((s, true)));
    }

    #[test]
    fn possession_needs_search() {
        let mut t = KeyTable::new();
        let mut c = caller();
        let s = session(&mut t, &mut c);
        let k = t.add(&c, s, KeyType::User, b"k", b"x".to_vec()).unwrap();
        assert!(t.possessed(&c, k));
        t.get_mut(k).unwrap().perm = 0x0100_0000;
        assert!(!t.possessed(&c, k));
        assert!(!t.permitted(k, &c, false, perm::VIEW));
        t.get_mut(k).unwrap().perm = 0x0900_0000;
        assert!(t.possessed(&c, k));
        assert!(t.permitted(k, &c, true, perm::VIEW));
        assert!(!t.permitted(k, &c, true, perm::WRITE));
        assert_eq!(t.search(&c, s, true, KeyType::User, b"k", false), Found::Key(k));
        t.get_mut(k).unwrap().perm = 0x0100_0000;
        assert_eq!(t.search(&c, s, true, KeyType::User, b"k", false), Found::Denied);
    }

    #[test]
    fn cycles_and_links() {
        let mut t = KeyTable::new();
        let mut c = caller();
        let s = session(&mut t, &mut c);
        let r1 = t.add(&c, s, KeyType::Keyring, b"r1", Vec::new()).unwrap();
        let r2 = t.add(&c, r1, KeyType::Keyring, b"r2", Vec::new()).unwrap();
        assert!(t.would_cycle(r1, r1));
        assert!(t.would_cycle(r2, r1));
        assert!(!t.would_cycle(r1, r2));
        let k = t.add(&c, r2, KeyType::User, b"deep", b"d".to_vec()).unwrap();
        assert!(!t.would_cycle(r2, k));
        assert_eq!(t.search(&c, s, true, KeyType::User, b"deep", false), Found::Key(k));
        assert_eq!(t.unlink(r2, k), Ok(()));
        assert_eq!(t.unlink(r2, k), Err(Errno::ENOENT));
    }

    #[test]
    fn request_and_negative_keys() {
        let mut t = KeyTable::new();
        let mut c = caller();
        let s = session(&mut t, &mut c);
        assert_eq!(t.request(&mut c, KeyType::User, b"miss", false, None), Err(ENOKEY));
        assert_eq!(t.request(&mut c, KeyType::User, b"miss", true, None), Err(Errno::ENOENT));
        assert_eq!(t.request(&mut c, KeyType::User, b"miss", true, None), Err(ENOKEY));
        assert_eq!(t.search(&c, s, true, KeyType::User, b"miss", false), Found::Negative);
        assert_eq!(t.request(&mut c, KeyType::Keyring, b"r", true, None), Err(Errno::EPERM));
        // The negative key times out.
        c.now += NEGATIVE_TIMEOUT;
        assert_eq!(t.request(&mut c, KeyType::User, b"miss", false, None), Err(ENOKEY));
        assert_eq!(t.request(&mut c, KeyType::User, b"miss", true, None), Err(Errno::ENOENT));
    }

    #[test]
    fn request_order_revoked_and_expired() {
        let mut t = KeyTable::new();
        let mut c = caller();
        let s = session(&mut t, &mut c);
        let k = t.add(&c, s, KeyType::User, b"k", b"x".to_vec()).unwrap();
        t.get_mut(k).unwrap().revoked = true;
        assert_eq!(t.request(&mut c, KeyType::User, b"k", false, None), Err(EKEYREVOKED));
        t.resolve(&mut c, spec::PROCESS_KEYRING, true).unwrap();
        assert_eq!(t.request(&mut c, KeyType::User, b"k", false, None), Err(ENOKEY), "an earlier keyring's miss wins");
        let e = t.add(&c, s, KeyType::User, b"e", b"x".to_vec()).unwrap();
        t.get_mut(e).unwrap().expiry = Some(c.now);
        assert_eq!(t.request(&mut c, KeyType::User, b"e", false, None), Err(ENOKEY));
        assert_eq!(t.search(&c, s, true, KeyType::User, b"e", false), Found::Expired);
    }

    #[test]
    fn invalidate_unlinks() {
        let mut t = KeyTable::new();
        let mut c = caller();
        let s = session(&mut t, &mut c);
        let k = t.add(&c, s, KeyType::User, b"k", b"x".to_vec()).unwrap();
        t.invalidate(k);
        assert!(t.get(k).is_none());
        assert!(t.get(s).unwrap().links.is_empty());
        assert_eq!(t.resolve(&mut c, k as i32, false), Err(ENOKEY));
    }

    #[test]
    fn join_named() {
        let mut t = KeyTable::new();
        let mut c = caller();
        let n = t.join(&mut c, Some(b"named"));
        assert_eq!(t.get(n).unwrap().perm, PERM_NAMED_SESSION);
        let mut other = caller();
        assert_eq!(t.join(&mut other, Some(b"named")), n, "the owner joins the same keyring");
        assert_ne!(t.join(&mut other, None), n);
    }

    #[test]
    fn type_rules() {
        assert_eq!(KeyType::Logon.check_description(b"svc:x"), Ok(()));
        assert_eq!(KeyType::Logon.check_description(b":x"), Err(Errno::EINVAL));
        assert_eq!(KeyType::Logon.check_description(b"x"), Err(Errno::EINVAL));
        assert_eq!(KeyType::Keyring.check_description(b".x"), Err(Errno::EPERM));
        assert_eq!(KeyType::User.check_payload(&[]), Err(Errno::EINVAL));
        assert_eq!(KeyType::User.check_payload(&[0; 32767]), Ok(()));
        assert_eq!(KeyType::User.check_payload(&[0; 32768]), Err(Errno::EINVAL));
        assert_eq!(KeyType::Keyring.check_payload(b"x"), Err(Errno::EINVAL));
        assert_eq!(
            ThreadKeys { thread: 1, process: 2, session: 3, reqkey_default: 4 }.for_fork(),
            ThreadKeys { thread: 0, process: 0, session: 3, reqkey_default: 4 }
        );
    }
}
