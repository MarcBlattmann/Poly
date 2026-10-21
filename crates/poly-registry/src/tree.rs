//! The key tree: keys, values, open instances, volatile keys, symbolic
//! links, deletion and last-write times.
//!
//! # Rules (Learn, and the guesses still to be confirmed)
//!
//! - Key and value names compare without case (Learn: "Structure of the
//!   Registry": "Each key has a name consisting of one or more printable
//!   characters. Key names are not case sensitive"), using the object
//!   manager's case folding ([`poly_ntobj::name::upcase`], ADR 0030). The
//!   case a name was created with is kept and reported.
//! - A key component is 1 to [`MAX_KEY_NAME_CHARS`] characters and holds no
//!   `\`; a value name is 0 to [`MAX_VALUE_NAME_CHARS`] (the empty name is
//!   the key's default value). Longer names are
//!   `STATUS_OBJECT_NAME_INVALID` and `STATUS_INVALID_PARAMETER`
//!   respectively (to be confirmed by the spec agent).
//! - `NtCreateKey` creates only the last component (Learn: "ZwCreateKey":
//!   "the new key must be a direct subkey"); a missing key on the way is
//!   `STATUS_OBJECT_NAME_NOT_FOUND` (to be confirmed).
//! - A volatile key (`REG_OPTION_VOLATILE`) is never saved; its subkeys
//!   must be volatile too (`STATUS_CHILD_MUST_BE_VOLATILE`). The master
//!   keys `\REGISTRY`, `\REGISTRY\MACHINE` and `\REGISTRY\USER` belong to
//!   no hive and count as volatile, so only hives (mounted with
//!   [`Registry::mount_hive`]) hold non-volatile keys.
//! - Opening an existing key with `NtCreateKey` changes nothing about it
//!   (Learn: "its value is not affected in any way"): not its class, not its
//!   volatility.
//! - Deleting a key with subkeys is `STATUS_CANNOT_DELETE` (Learn:
//!   "ZwDeleteKey"), and so is deleting a master key or a hive root (to be
//!   confirmed). A deleted key leaves the namespace at once; its open
//!   instances stay valid handles on which everything but closing fails
//!   with `STATUS_KEY_DELETED`.
//! - Subkeys enumerate sorted by upcased name, values in the order they
//!   were first set (Learn says neither is ordered; the order is Poly's).
//! - Last-write time: set when the key is created, and by every change of
//!   its values, of its list of subkeys, and by `KeyWriteTimeInformation`
//!   (Learn: "KEY_FULL_INFORMATION": "The last time this key or any of its
//!   values changed"; subkey changes counting is to be confirmed). Writing a
//!   value with the type and data it already has changes nothing and
//!   notifies nobody (Learn: "ZwNotifyChangeKey": "The caller receives no
//!   notification if the new value written to the key matches the previous
//!   value").
//! - Symbolic links (Learn: "RegCreateKeyExW", `REG_OPTION_CREATE_LINK`):
//!   a link key's target is its `SymbolicLinkValue` value of type
//!   `REG_LINK`, a full NT path (`\REGISTRY\MACHINE\...`). Walking through a
//!   link restarts the lookup at the target with the rest of the path
//!   appended ([`Resolved::Reparse`]); a link as the last component is
//!   followed unless the open asks for the link itself (`OBJ_OPENLINK`,
//!   `REG_OPTION_OPEN_LINK`). A link without a usable target (no value, not
//!   `REG_LINK`, not starting with `\`) is `STATUS_OBJECT_TYPE_MISMATCH`
//!   (Learn: "ZwOpenKeyEx" lists that status for a symbolic link opened
//!   without `REG_OPTION_OPEN_LINK`); a link has no subkeys.

use crate::consts::*;
use crate::hive::{HiveFile, KeyImage};
use crate::slab::Slab;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::name::{SEPARATOR, upcase, wide};
use poly_ntobj::status::*;

pub type Result<T> = core::result::Result<T, NtStatus>;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name {
            pub(crate) index: u32,
            pub(crate) generation: u32,
        }

        impl $name {
            /// As one number (for an object's cookie).
            pub const fn to_bits(self) -> u64 {
                (self.generation as u64) << 32 | self.index as u64
            }

            pub const fn from_bits(bits: u64) -> $name {
                $name { index: bits as u32, generation: (bits >> 32) as u32 }
            }

            pub(crate) const fn new((index, generation): (u32, u32)) -> $name {
                $name { index, generation }
            }
        }
    };
}

id_type!(
    /// Names a key. Carries a generation, so an id kept after the key was
    /// freed never reaches a new key in the same slot.
    KeyId
);
id_type!(
    /// One open instance of a key: what a key object (and its handles)
    /// stands for. Notifications belong to it (Learn: "ZwNotifyChangeKey":
    /// "The notification session ends when KeyHandle is closed").
    OpenId
);

/// A hive: the subtree below a mount point that is saved to one file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct HiveId(pub u32);

/// A value entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    pub name: Vec<u16>,
    /// `REG_*` type. Any number is allowed (Learn: "ZwSetValueKey" lists the
    /// system-defined ones; the registry stores the type it is given).
    pub kind: u32,
    pub data: Vec<u8>,
}

/// A key.
#[derive(Clone, Debug)]
pub struct Key {
    pub(crate) name: Vec<u16>,
    pub(crate) class: Vec<u16>,
    pub(crate) parent: Option<KeyId>,
    /// Upcased name to subkey.
    pub(crate) subkeys: BTreeMap<Vec<u16>, KeyId>,
    pub(crate) values: Vec<Value>,
    pub(crate) last_write: u64,
    pub(crate) volatile: bool,
    pub(crate) link: bool,
    pub(crate) hive: Option<HiveId>,
    pub(crate) deleted: bool,
    /// Open instances ([`OpenId`]) of this key.
    pub(crate) opens: u32,
    pub(crate) depth: usize,
}

impl Key {
    pub fn name(&self) -> &[u16] {
        &self.name
    }

    pub fn class(&self) -> &[u16] {
        &self.class
    }

    pub fn last_write(&self) -> u64 {
        self.last_write
    }

    pub fn is_volatile(&self) -> bool {
        self.volatile
    }

    pub fn is_link(&self) -> bool {
        self.link
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    pub fn hive(&self) -> Option<HiveId> {
        self.hive
    }

    pub fn subkey_count(&self) -> usize {
        self.subkeys.len()
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }
}

/// What `NtCreateKey` asks for a new key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreateOptions<'a> {
    /// `REG_OPTION_VOLATILE`.
    pub volatile: bool,
    /// `REG_OPTION_CREATE_LINK`.
    pub link: bool,
    /// The `Class` string.
    pub class: &'a [u16],
}

/// Whether `NtCreateKey` made the key (`REG_CREATED_NEW_KEY`) or opened it
/// (`REG_OPENED_EXISTING_KEY`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Created,
    Opened,
}

impl Disposition {
    pub const fn value(self) -> u32 {
        match self {
            Disposition::Created => REG_CREATED_NEW_KEY,
            Disposition::Opened => REG_OPENED_EXISTING_KEY,
        }
    }
}

/// Where a path led ([`Registry::resolve`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// An existing key.
    Found(KeyId),
    /// The last component does not exist; `parent` is where it would go.
    Missing { parent: KeyId, name: Vec<u16> },
    /// A symbolic link on the way: start over with this full NT path.
    Reparse(Vec<u16>),
}

/// Counts and maximum lengths of a key, for the information classes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyInfo {
    pub name: Vec<u16>,
    pub class: Vec<u16>,
    pub last_write: u64,
    pub subkeys: u32,
    /// Bytes of the longest subkey name.
    pub max_name_len: u32,
    /// Bytes of the longest subkey class.
    pub max_class_len: u32,
    pub values: u32,
    /// Bytes of the longest value name.
    pub max_value_name_len: u32,
    /// Bytes of the largest value data.
    pub max_value_data_len: u32,
}

/// A mounted hive.
#[derive(Clone, Debug)]
pub struct Hive {
    /// The file name it is saved under (no directory, no extension).
    pub file: String,
    pub root: KeyId,
    /// Changed since it was last saved (or never saved).
    pub dirty: bool,
    /// Incremented by every save.
    pub sequence: u64,
}

/// An open instance.
#[derive(Clone, Debug)]
pub(crate) struct Open {
    pub(crate) key: KeyId,
    pub(crate) watch: Option<crate::notify::Watch>,
}

/// The registry: the key tree below `\REGISTRY`, its hives, open
/// instances and pending notifications. The kernel keeps one.
#[derive(Clone, Debug)]
pub struct Registry {
    pub(crate) keys: Slab<Key>,
    pub(crate) opens: Slab<Open>,
    root: KeyId,
    machine: KeyId,
    user: KeyId,
    pub(crate) hives: Vec<Hive>,
    pub(crate) fired: VecDeque<crate::notify::Fired>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Upcase a name, as subkeys are indexed.
pub fn fold(name: &[u16]) -> Vec<u16> {
    name.iter().map(|&c| upcase(c)).collect()
}

fn eq_fold(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| x == y || upcase(x) == upcase(y))
}

fn valid_component(name: &[u16]) -> Result<()> {
    if name.is_empty() || name.len() > MAX_KEY_NAME_CHARS || name.contains(&SEPARATOR) {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    Ok(())
}

/// The name of the root key and of the two master keys below it, as
/// `KeyNameInformation` reports them (to be confirmed by the spec agent:
/// Learn writes `\Registry\Machine`; lookups ignore case either way).
pub const ROOT_NAME: &str = "REGISTRY";
pub const MACHINE_NAME: &str = "MACHINE";
pub const USER_NAME: &str = "USER";

impl Registry {
    /// `\REGISTRY` with its two master keys, `MACHINE` and `USER`, all
    /// volatile and in no hive.
    pub fn new() -> Registry {
        let mut r = Registry {
            keys: Slab::new(),
            opens: Slab::new(),
            root: KeyId::new((0, 0)),
            machine: KeyId::new((0, 0)),
            user: KeyId::new((0, 0)),
            hives: Vec::new(),
            fired: VecDeque::new(),
        };
        r.root = r.insert_key(wide(ROOT_NAME), None, true, None, 0);
        r.machine = r.insert_key(wide(MACHINE_NAME), Some(r.root), true, None, 0);
        r.user = r.insert_key(wide(USER_NAME), Some(r.root), true, None, 0);
        r
    }

    fn insert_key(
        &mut self,
        name: Vec<u16>,
        parent: Option<KeyId>,
        volatile: bool,
        hive: Option<HiveId>,
        now: u64,
    ) -> KeyId {
        let depth = parent.map_or(0, |p| self.raw(p).depth + 1);
        let folded = fold(&name);
        let key = Key {
            name,
            class: Vec::new(),
            parent,
            subkeys: BTreeMap::new(),
            values: Vec::new(),
            last_write: now,
            volatile,
            link: false,
            hive,
            deleted: false,
            opens: 0,
            depth,
        };
        let id = KeyId::new(self.keys.insert(key));
        if let Some(p) = parent {
            self.raw_mut(p).subkeys.insert(folded, id);
        }
        id
    }

    /// `\REGISTRY`.
    pub fn root(&self) -> KeyId {
        self.root
    }

    /// `\REGISTRY\MACHINE` (`HKEY_LOCAL_MACHINE`).
    pub fn machine(&self) -> KeyId {
        self.machine
    }

    /// `\REGISTRY\USER` (`HKEY_USERS`).
    pub fn user(&self) -> KeyId {
        self.user
    }

    pub(crate) fn raw(&self, id: KeyId) -> &Key {
        self.keys.get(id.index, id.generation).expect("live key id")
    }

    pub(crate) fn raw_mut(&mut self, id: KeyId) -> &mut Key {
        self.keys.get_mut(id.index, id.generation).expect("live key id")
    }

    /// A key that exists and was not deleted: else `STATUS_KEY_DELETED`
    /// (also for an id whose key was freed).
    pub fn key(&self, id: KeyId) -> Result<&Key> {
        match self.keys.get(id.index, id.generation) {
            Some(k) if !k.deleted => Ok(k),
            _ => Err(STATUS_KEY_DELETED),
        }
    }

    fn key_mut(&mut self, id: KeyId) -> Result<&mut Key> {
        match self.keys.get_mut(id.index, id.generation) {
            Some(k) if !k.deleted => Ok(k),
            _ => Err(STATUS_KEY_DELETED),
        }
    }

    /// Keys stored, including deleted ones still open.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Open instances.
    pub fn open_count(&self) -> usize {
        self.opens.len()
    }

    /// The subkey of `key` called `name` (any case).
    pub fn child(&self, key: KeyId, name: &[u16]) -> Option<KeyId> {
        self.keys.get(key.index, key.generation)?.subkeys.get(&fold(name)).copied()
    }

    /// The key at `path` below `start` (components separated by `\`, a
    /// leading `\` ignored), following links. For the kernel's own lookups
    /// and tests; services go through [`resolve`](Self::resolve).
    pub fn find(&self, start: KeyId, path: &str) -> Option<KeyId> {
        let mut cur = start;
        for c in path.split('\\').filter(|c| !c.is_empty()) {
            cur = self.child(cur, &wide(c))?;
        }
        Some(cur)
    }

    /// The full NT path of a key: `\REGISTRY\MACHINE\SOFTWARE\...`.
    pub fn full_path(&self, id: KeyId) -> Result<Vec<u16>> {
        self.key(id)?;
        let mut parts = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let k = self.raw(c);
            parts.push(c);
            cur = k.parent;
        }
        let mut out = Vec::new();
        for c in parts.iter().rev() {
            out.push(SEPARATOR);
            out.extend_from_slice(&self.raw(*c).name);
        }
        Ok(out)
    }

    /// Whether `ancestor` is `key` or above it.
    pub(crate) fn is_at_or_below(&self, key: KeyId, ancestor: KeyId) -> bool {
        let mut cur = Some(key);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.keys.get(c.index, c.generation).and_then(|k| k.parent);
        }
        false
    }

    // ----- lookups

    /// The target of a link key, as a full NT path.
    pub fn link_target(&self, id: KeyId) -> Result<Vec<u16>> {
        let k = self.key(id)?;
        let v = k
            .values
            .iter()
            .find(|v| eq_fold(&v.name, &wide(SYMBOLIC_LINK_VALUE)))
            .filter(|v| v.kind == REG_LINK && v.data.len() % 2 == 0)
            .ok_or(STATUS_OBJECT_TYPE_MISMATCH)?;
        let mut target: Vec<u16> = v.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        while target.last() == Some(&0) {
            target.pop();
        }
        if target.first() != Some(&SEPARATOR) {
            return Err(STATUS_OBJECT_TYPE_MISMATCH);
        }
        Ok(target)
    }

    fn reparse(&self, link: KeyId, rest: &[&[u16]]) -> Result<Resolved> {
        let mut path = self.link_target(link)?;
        for c in rest {
            path.push(SEPARATOR);
            path.extend_from_slice(c);
        }
        Ok(Resolved::Reparse(path))
    }

    /// Walk `path` from `start`: components separated by `\` (one leading
    /// `\` is skipped: the rest of a path after a parse object starts with
    /// one). `open_link` keeps a link as the last component instead of
    /// following it. An empty component (`a\\b`, a trailing `\`) is
    /// `STATUS_OBJECT_NAME_INVALID`; a missing key on the way
    /// `STATUS_OBJECT_NAME_NOT_FOUND`; a deleted `start`
    /// `STATUS_KEY_DELETED`.
    pub fn resolve(&self, start: KeyId, path: &[u16], open_link: bool) -> Result<Resolved> {
        self.key(start)?;
        let path = path.strip_prefix(&[SEPARATOR]).unwrap_or(path);
        if path.is_empty() {
            return Ok(Resolved::Found(start));
        }
        let parts: Vec<&[u16]> = path.split(|&c| c == SEPARATOR).collect();
        if parts.iter().any(|p| p.is_empty()) {
            return Err(STATUS_OBJECT_NAME_INVALID);
        }
        if self.raw(start).link {
            // A path below a link opened as itself: below its target.
            return self.reparse(start, &parts);
        }
        let mut cur = start;
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            if part.len() > MAX_KEY_NAME_CHARS {
                return Err(STATUS_OBJECT_NAME_INVALID);
            }
            match self.child(cur, part) {
                None if last => return Ok(Resolved::Missing { parent: cur, name: part.to_vec() }),
                None => return Err(STATUS_OBJECT_NAME_NOT_FOUND),
                Some(c) if self.raw(c).link && !(last && open_link) => return self.reparse(c, &parts[i + 1..]),
                Some(c) => cur = c,
            }
        }
        Ok(Resolved::Found(cur))
    }

    // ----- open instances

    /// A new open instance of `key` (a key object).
    pub fn open(&mut self, key: KeyId) -> Result<OpenId> {
        self.key_mut(key)?.opens += 1;
        Ok(OpenId::new(self.opens.insert(Open { key, watch: None })))
    }

    /// The key an open instance is of: `STATUS_KEY_DELETED` once it was
    /// deleted, `STATUS_INVALID_HANDLE` for a closed instance.
    pub fn key_of(&self, open: OpenId) -> Result<KeyId> {
        let o = self.opens.get(open.index, open.generation).ok_or(STATUS_INVALID_HANDLE)?;
        self.key(o.key)?;
        Ok(o.key)
    }

    /// End an open instance (its key object was deleted): its pending
    /// notifications complete with `STATUS_NOTIFY_CLEANUP` (Learn:
    /// "RegNotifyChangeKeyValue": "If the specified key is closed, the event
    /// is signaled"), and a deleted key with no instance left is freed.
    pub fn close(&mut self, open: OpenId) {
        let Some(o) = self.opens.remove(open.index, open.generation) else { return };
        if let Some(w) = o.watch {
            for request in w.pending {
                self.fired.push_back(crate::notify::Fired { open, request, status: STATUS_NOTIFY_CLEANUP });
            }
        }
        let k = self.raw_mut(o.key);
        k.opens -= 1;
        if k.deleted && k.opens == 0 {
            self.keys.remove(o.key.index, o.key.generation);
        }
    }

    // ----- changes

    fn mark_dirty(&mut self, id: KeyId) {
        let k = self.raw(id);
        if let (false, Some(h)) = (k.volatile, k.hive) {
            self.hives[h.0 as usize].dirty = true;
        }
    }

    /// `NtCreateKey`'s work once the walk found the last component free (or
    /// taken: then the existing key is opened, unless `opts.link` asks for a
    /// new link, which collides with `STATUS_OBJECT_NAME_COLLISION`, to be
    /// confirmed by the spec agent).
    pub fn create_key(
        &mut self,
        parent: KeyId,
        name: &[u16],
        opts: CreateOptions<'_>,
        now: u64,
    ) -> Result<(KeyId, Disposition)> {
        let p = self.key(parent)?;
        valid_component(name)?;
        if let Some(existing) = self.child(parent, name) {
            if opts.link {
                return Err(STATUS_OBJECT_NAME_COLLISION);
            }
            return Ok((existing, Disposition::Opened));
        }
        if p.link {
            // A link has no subkeys (to be confirmed by the spec agent).
            return Err(STATUS_ACCESS_DENIED);
        }
        if p.volatile && !opts.volatile {
            return Err(STATUS_CHILD_MUST_BE_VOLATILE);
        }
        if p.depth + 1 > MAX_DEPTH {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let hive = p.hive;
        let id = self.insert_key(name.to_vec(), Some(parent), opts.volatile, hive, now);
        let k = self.raw_mut(id);
        k.class = opts.class.to_vec();
        k.link = opts.link;
        self.raw_mut(parent).last_write = now;
        // The parent's hive changes only if the new key is saved with it.
        self.mark_dirty(id);
        self.changed(parent, REG_NOTIFY_CHANGE_NAME);
        Ok((id, Disposition::Created))
    }

    /// `NtDeleteKey`.
    pub fn delete_key(&mut self, id: KeyId, now: u64) -> Result<()> {
        let k = self.key(id)?;
        if !k.subkeys.is_empty() {
            return Err(STATUS_CANNOT_DELETE);
        }
        let is_hive_root = self.hives.iter().any(|h| h.root == id);
        let Some(parent) = k.parent.filter(|_| !is_hive_root && id != self.machine && id != self.user) else {
            return Err(STATUS_CANNOT_DELETE);
        };
        let folded = fold(&k.name);
        self.mark_dirty(id);
        self.complete_watches_of(id, STATUS_KEY_DELETED);
        let p = self.raw_mut(parent);
        p.subkeys.remove(&folded);
        p.last_write = now;
        let k = self.raw_mut(id);
        k.deleted = true;
        k.values.clear();
        if k.opens == 0 {
            self.keys.remove(id.index, id.generation);
        }
        self.changed(parent, REG_NOTIFY_CHANGE_NAME);
        Ok(())
    }

    /// `NtSetValueKey`: create or replace the value `name`.
    pub fn set_value(&mut self, id: KeyId, name: &[u16], kind: u32, data: &[u8], now: u64) -> Result<()> {
        if name.len() > MAX_VALUE_NAME_CHARS {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let k = self.key_mut(id)?;
        match k.values.iter_mut().find(|v| eq_fold(&v.name, name)) {
            Some(v) if v.kind == kind && v.data == data => return Ok(()),
            Some(v) => {
                v.kind = kind;
                v.data = data.to_vec();
            }
            None => k.values.push(Value { name: name.to_vec(), kind, data: data.to_vec() }),
        }
        k.last_write = now;
        self.mark_dirty(id);
        self.changed(id, REG_NOTIFY_CHANGE_LAST_SET);
        Ok(())
    }

    /// `NtDeleteValueKey`: `STATUS_OBJECT_NAME_NOT_FOUND` if there is no
    /// such value (Learn).
    pub fn delete_value(&mut self, id: KeyId, name: &[u16], now: u64) -> Result<()> {
        let k = self.key_mut(id)?;
        let i = k.values.iter().position(|v| eq_fold(&v.name, name)).ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
        k.values.remove(i);
        k.last_write = now;
        self.mark_dirty(id);
        self.changed(id, REG_NOTIFY_CHANGE_LAST_SET);
        Ok(())
    }

    /// `NtSetInformationKey(KeyWriteTimeInformation)`. Notifies
    /// `REG_NOTIFY_CHANGE_ATTRIBUTES` watchers (to be confirmed by the spec
    /// agent).
    pub fn set_last_write(&mut self, id: KeyId, time: u64) -> Result<()> {
        self.key_mut(id)?.last_write = time;
        self.mark_dirty(id);
        self.changed(id, REG_NOTIFY_CHANGE_ATTRIBUTES);
        Ok(())
    }

    // ----- queries

    /// The value called `name` (any case): `STATUS_OBJECT_NAME_NOT_FOUND`
    /// if there is none (Learn: "ZwQueryValueKey").
    pub fn value(&self, id: KeyId, name: &[u16]) -> Result<&Value> {
        self.key(id)?.values.iter().find(|v| eq_fold(&v.name, name)).ok_or(STATUS_OBJECT_NAME_NOT_FOUND)
    }

    /// Value number `index`: `STATUS_NO_MORE_ENTRIES` past the last
    /// (Learn: "ZwEnumerateValueKey").
    pub fn value_at(&self, id: KeyId, index: usize) -> Result<&Value> {
        self.key(id)?.values.get(index).ok_or(STATUS_NO_MORE_ENTRIES)
    }

    /// Subkey number `index`: `STATUS_NO_MORE_ENTRIES` past the last
    /// (Learn: "ZwEnumerateKey").
    pub fn subkey_at(&self, id: KeyId, index: usize) -> Result<KeyId> {
        self.key(id)?.subkeys.values().nth(index).copied().ok_or(STATUS_NO_MORE_ENTRIES)
    }

    /// Counts and maximum lengths (`KEY_FULL_INFORMATION`,
    /// `KEY_CACHED_INFORMATION`).
    pub fn info(&self, id: KeyId) -> Result<KeyInfo> {
        let k = self.key(id)?;
        let subs = k.subkeys.values().map(|s| self.raw(*s));
        let (mut max_name, mut max_class) = (0, 0);
        for s in subs {
            max_name = max_name.max(s.name.len() * 2);
            max_class = max_class.max(s.class.len() * 2);
        }
        Ok(KeyInfo {
            name: k.name.clone(),
            class: k.class.clone(),
            last_write: k.last_write,
            subkeys: k.subkeys.len() as u32,
            max_name_len: max_name as u32,
            max_class_len: max_class as u32,
            values: k.values.len() as u32,
            max_value_name_len: k.values.iter().map(|v| v.name.len() * 2).max().unwrap_or(0) as u32,
            max_value_data_len: k.values.iter().map(|v| v.data.len()).max().unwrap_or(0) as u32,
        })
    }

    // ----- hives

    /// Mount a hive as the non-volatile key `name` below `parent` (a master
    /// key), filled from a loaded hive file (its root's name is ignored) or
    /// empty. An empty hive starts dirty, so the first save writes it.
    pub fn mount_hive(
        &mut self,
        parent: KeyId,
        name: &[u16],
        file: &str,
        loaded: Option<&HiveFile>,
        now: u64,
    ) -> Result<HiveId> {
        self.key(parent)?;
        valid_component(name)?;
        if self.child(parent, name).is_some() {
            return Err(STATUS_OBJECT_NAME_COLLISION);
        }
        let hive = HiveId(self.hives.len() as u32);
        let root = self.insert_key(name.to_vec(), Some(parent), false, Some(hive), now);
        let sequence = loaded.map_or(0, |l| l.sequence);
        self.hives.push(Hive { file: file.into(), root, dirty: loaded.is_none(), sequence });
        if let Some(img) = loaded.map(|l| &l.root) {
            let k = self.raw_mut(root);
            k.class = img.class.clone();
            k.last_write = img.last_write;
            k.values = img.values.clone();
            for sub in &img.subkeys {
                self.insert_image(root, sub, hive);
            }
        }
        Ok(hive)
    }

    fn insert_image(&mut self, parent: KeyId, img: &KeyImage, hive: HiveId) {
        let id = self.insert_key(img.name.clone(), Some(parent), false, Some(hive), img.last_write);
        let k = self.raw_mut(id);
        k.class = img.class.clone();
        k.link = img.link;
        k.values = img.values.clone();
        for sub in &img.subkeys {
            self.insert_image(id, sub, hive);
        }
    }

    pub fn hives(&self) -> &[Hive] {
        &self.hives
    }

    pub fn hive(&self, id: HiveId) -> &Hive {
        &self.hives[id.0 as usize]
    }

    /// The hive saved under `file`.
    pub fn hive_by_file(&self, file: &str) -> Option<HiveId> {
        self.hives.iter().position(|h| h.file == file).map(|i| HiveId(i as u32))
    }

    /// The non-volatile contents of a hive.
    pub fn snapshot(&self, hive: HiveId) -> KeyImage {
        self.image_of(self.hives[hive.0 as usize].root)
    }

    fn image_of(&self, id: KeyId) -> KeyImage {
        let k = self.raw(id);
        KeyImage {
            name: k.name.clone(),
            class: k.class.clone(),
            last_write: k.last_write,
            link: k.link,
            values: k.values.clone(),
            subkeys: k.subkeys.values().filter(|s| !self.raw(**s).volatile).map(|s| self.image_of(*s)).collect(),
        }
    }

    /// The hive file for `hive` ([`crate::hive::encode`]), marking it clean
    /// and counting the save. If writing it fails, call
    /// [`mark_hive_dirty`](Self::mark_hive_dirty).
    pub fn save(&mut self, hive: HiveId) -> Vec<u8> {
        let root = self.snapshot(hive);
        let h = &mut self.hives[hive.0 as usize];
        h.sequence += 1;
        h.dirty = false;
        crate::hive::encode(&HiveFile { sequence: h.sequence, root })
    }

    pub fn mark_hive_dirty(&mut self, hive: HiveId) {
        self.hives[hive.0 as usize].dirty = true;
    }

    /// Hives changed since their last save.
    pub fn dirty_hives(&self) -> Vec<HiveId> {
        (0..self.hives.len() as u32).map(HiveId).filter(|h| self.hives[h.0 as usize].dirty).collect()
    }
}
