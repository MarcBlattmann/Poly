//! bpf(2) maps and programs (ADR 0135): what the system call keeps behind
//! its descriptors. The syscall layer is [`crate::sys::bpf`].
//!
//! Poly has no BPF interpreter or JIT, so only what needs none exists:
//! array and hash maps, whose elements live in memory and are read and
//! written through the descriptor, and the loading of straight-line
//! socket filters that pass a small verifier (they are never run).

use crate::errno::Errno;
use crate::ns::lock::Mutex;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

/// `BPF_MAP_TYPE_HASH` and `BPF_MAP_TYPE_ARRAY` (`linux/bpf.h`).
pub const MAP_TYPE_HASH: u32 = 1;
pub const MAP_TYPE_ARRAY: u32 = 2;
/// `BPF_PROG_TYPE_SOCKET_FILTER`: the one program type Poly loads (every
/// other, known or not, is `EINVAL`).
pub const PROG_TYPE_SOCKET_FILTER: u32 = 1;

/// The size Poly gives `union bpf_attr`: the largest member of the header
/// (`BPF_PROG_LOAD`). Bytes a caller passes past it must be zero.
pub const ATTR_SIZE: usize = 160;
/// `sizeof(struct bpf_map_info)` of the reference kernel.
pub const MAP_INFO_SIZE: usize = 104;
/// `sizeof(struct bpf_prog_info)` of the reference kernel.
pub const PROG_INFO_SIZE: usize = 232;
/// `BPF_OBJ_NAME_LEN`.
pub const NAME_LEN: usize = 16;

/// Map flags (`linux/bpf.h`: `BPF_F_*`).
pub mod flag {
    pub const NO_PREALLOC: u32 = 1 << 0;
    pub const NUMA_NODE: u32 = 1 << 2;
    pub const RDONLY: u32 = 1 << 3;
    pub const WRONLY: u32 = 1 << 4;
    pub const ZERO_SEED: u32 = 1 << 6;
    pub const RDONLY_PROG: u32 = 1 << 7;
    pub const WRONLY_PROG: u32 = 1 << 8;
    pub const MMAPABLE: u32 = 1 << 10;
    pub const PRESERVE_ELEMS: u32 = 1 << 11;
    pub const INNER_MAP: u32 = 1 << 12;
    /// What may be set at all (also the flags no map of Poly uses).
    pub const KNOWN: u32 = (1 << 15) - 1;
}

/// `BPF_ANY`, `BPF_NOEXIST`, `BPF_EXIST`.
pub const ANY: u64 = 0;
pub const NOEXIST: u64 = 1;
pub const EXIST: u64 = 2;

/// The largest value of a map (the reference allocates it in one piece).
const VALUE_MAX: u32 = 4 << 20;
/// The largest key of a hash map.
const KEY_MAX: u32 = 1 << 20;
/// The most instructions a program has (`BPF_COMPLEXITY_LIMIT_INSNS`).
pub const INSN_MAX: usize = 1_000_000;

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn next_id() -> u32 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// The leading fields of `BPF_MAP_CREATE`'s attribute.
#[derive(Clone, Copy, Debug, Default)]
pub struct MapAttr {
    pub map_type: u32,
    pub key_size: u32,
    pub value_size: u32,
    pub max_entries: u32,
    pub map_flags: u32,
    pub name: [u8; NAME_LEN],
    pub map_extra: u64,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

impl MapAttr {
    /// From the attribute bytes (at least [`ATTR_SIZE`], zero-filled).
    pub fn parse(b: &[u8]) -> MapAttr {
        let mut name = [0u8; NAME_LEN];
        name.copy_from_slice(&b[28..28 + NAME_LEN]);
        MapAttr {
            map_type: u32_at(b, 0),
            key_size: u32_at(b, 4),
            value_size: u32_at(b, 8),
            max_entries: u32_at(b, 12),
            map_flags: u32_at(b, 16),
            name,
            map_extra: u64_at(b, 64),
        }
    }
}

/// An object name: up to 15 characters of `[A-Za-z0-9_.]` and a NUL.
pub fn name_ok(name: &[u8; NAME_LEN]) -> bool {
    let Some(end) = name.iter().position(|&c| c == 0) else { return false };
    name[..end].iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
}

#[derive(Debug)]
enum Elements {
    /// Index to value; missing indexes are zero.
    Array(BTreeMap<u32, Vec<u8>>),
    Hash(BTreeMap<Vec<u8>, Vec<u8>>),
}

#[derive(Debug)]
struct MapState {
    frozen: bool,
    elements: Elements,
}

/// An array or hash map.
#[derive(Debug)]
pub struct Map {
    pub id: u32,
    pub map_type: u32,
    pub key_size: u32,
    pub value_size: u32,
    pub max_entries: u32,
    pub flags: u32,
    pub name: [u8; NAME_LEN],
    state: Mutex<MapState>,
}

impl Map {
    /// `BPF_MAP_CREATE`: the attribute's rules (`EINVAL` for most, `E2BIG`
    /// for sizes no allocation could hold).
    pub fn create(a: &MapAttr) -> Result<Arc<Map>, Errno> {
        let allowed = match a.map_type {
            MAP_TYPE_HASH => {
                flag::NO_PREALLOC
                    | flag::NUMA_NODE
                    | flag::ZERO_SEED
                    | flag::RDONLY
                    | flag::WRONLY
                    | flag::RDONLY_PROG
                    | flag::WRONLY_PROG
                    | flag::INNER_MAP
            }
            MAP_TYPE_ARRAY => {
                flag::NUMA_NODE
                    | flag::RDONLY
                    | flag::WRONLY
                    | flag::RDONLY_PROG
                    | flag::WRONLY_PROG
                    | flag::MMAPABLE
                    | flag::PRESERVE_ELEMS
                    | flag::INNER_MAP
            }
            _ => return Err(Errno::EINVAL),
        };
        if a.key_size == 0 || a.value_size == 0 || a.max_entries == 0 || a.map_extra != 0 {
            return Err(Errno::EINVAL);
        }
        if a.map_type == MAP_TYPE_ARRAY && a.key_size != 4 {
            return Err(Errno::EINVAL);
        }
        if a.map_flags & !allowed != 0 || a.map_flags & flag::RDONLY != 0 && a.map_flags & flag::WRONLY != 0 {
            return Err(Errno::EINVAL);
        }
        if !name_ok(&a.name) {
            return Err(Errno::EINVAL);
        }
        if a.value_size > VALUE_MAX || a.key_size > KEY_MAX {
            return Err(Errno::E2BIG);
        }
        let elements = if a.map_type == MAP_TYPE_ARRAY {
            Elements::Array(BTreeMap::new())
        } else {
            Elements::Hash(BTreeMap::new())
        };
        Ok(Arc::new(Map {
            id: next_id(),
            map_type: a.map_type,
            key_size: a.key_size,
            value_size: a.value_size,
            max_entries: a.max_entries,
            flags: a.map_flags,
            name: a.name,
            state: Mutex::new(MapState { frozen: false, elements }),
        }))
    }

    /// The array index a key bytes name.
    fn index(key: &[u8]) -> u32 {
        u32::from_le_bytes(key[..4].try_into().expect("4-byte key"))
    }

    /// `BPF_MAP_LOOKUP_ELEM`. `key` is `key_size` bytes.
    pub fn lookup(&self, key: &[u8]) -> Result<Vec<u8>, Errno> {
        if self.flags & flag::WRONLY != 0 {
            return Err(Errno::EPERM);
        }
        let st = self.state.lock();
        match &st.elements {
            Elements::Array(m) => {
                let i = Self::index(key);
                if i >= self.max_entries {
                    return Err(Errno::ENOENT);
                }
                Ok(m.get(&i).cloned().unwrap_or_else(|| vec![0; self.value_size as usize]))
            }
            Elements::Hash(m) => m.get(key).cloned().ok_or(Errno::ENOENT),
        }
    }

    /// `BPF_MAP_UPDATE_ELEM` with `flags` ([`ANY`], [`NOEXIST`], [`EXIST`]).
    pub fn update(&self, key: &[u8], value: &[u8], flags: u64) -> Result<(), Errno> {
        if flags > EXIST {
            return Err(Errno::EINVAL);
        }
        if self.flags & flag::RDONLY != 0 {
            return Err(Errno::EPERM);
        }
        let mut st = self.state.lock();
        if st.frozen {
            return Err(Errno::EPERM);
        }
        match &mut st.elements {
            Elements::Array(m) => {
                let i = Self::index(key);
                if i >= self.max_entries {
                    return Err(Errno::E2BIG);
                }
                if flags == NOEXIST {
                    return Err(Errno::EEXIST);
                }
                m.insert(i, value.to_vec());
            }
            Elements::Hash(m) => {
                let present = m.contains_key(key);
                if flags == NOEXIST && present {
                    return Err(Errno::EEXIST);
                }
                if flags == EXIST && !present {
                    return Err(Errno::ENOENT);
                }
                if !present && m.len() >= self.max_entries as usize {
                    return Err(Errno::E2BIG);
                }
                m.insert(key.to_vec(), value.to_vec());
            }
        }
        Ok(())
    }

    /// `BPF_MAP_DELETE_ELEM`.
    pub fn delete(&self, key: &[u8]) -> Result<(), Errno> {
        if self.flags & flag::RDONLY != 0 {
            return Err(Errno::EPERM);
        }
        let mut st = self.state.lock();
        if st.frozen {
            return Err(Errno::EPERM);
        }
        match &mut st.elements {
            Elements::Array(_) => Err(Errno::EINVAL),
            Elements::Hash(m) => m.remove(key).map(|_| ()).ok_or(Errno::ENOENT),
        }
    }

    /// `BPF_MAP_GET_NEXT_KEY`: the key after `key`; the first one for no
    /// key or one that is not in the map (array: an index out of range).
    /// `ENOENT` past the last.
    pub fn next_key(&self, key: Option<&[u8]>) -> Result<Vec<u8>, Errno> {
        let st = self.state.lock();
        match &st.elements {
            Elements::Array(_) => {
                let next = match key.map(Self::index) {
                    None => 0,
                    Some(i) if i >= self.max_entries => 0,
                    Some(i) if i + 1 >= self.max_entries => return Err(Errno::ENOENT),
                    Some(i) => i + 1,
                };
                Ok(next.to_le_bytes().to_vec())
            }
            Elements::Hash(m) => {
                use core::ops::Bound::{Excluded, Unbounded};
                let first = || m.keys().next().cloned().ok_or(Errno::ENOENT);
                match key {
                    Some(k) if m.contains_key(k) => {
                        m.range::<[u8], _>((Excluded(k), Unbounded)).next().map(|(k, _)| k.clone()).ok_or(Errno::ENOENT)
                    }
                    _ => first(),
                }
            }
        }
    }

    /// `BPF_MAP_FREEZE`: user space can no longer change the map; freezing
    /// again is `EPERM`.
    pub fn freeze(&self) -> Result<(), Errno> {
        let mut st = self.state.lock();
        if st.frozen {
            return Err(Errno::EPERM);
        }
        st.frozen = true;
        Ok(())
    }

    /// `struct bpf_map_info`: type, id, sizes, flags and the name.
    pub fn info(&self) -> [u8; MAP_INFO_SIZE] {
        let mut b = [0u8; MAP_INFO_SIZE];
        for (i, v) in
            [self.map_type, self.id, self.key_size, self.value_size, self.max_entries, self.flags].iter().enumerate()
        {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[24..24 + NAME_LEN].copy_from_slice(&self.name);
        b
    }

    /// The map's lines of `/proc/<pid>/fdinfo/<fd>`.
    pub fn fdinfo(&self) -> String {
        let frozen = u32::from(self.state.lock().frozen);
        format!(
            "map_type:\t{}\nkey_size:\t{}\nvalue_size:\t{}\nmax_entries:\t{}\nmap_flags:\t{:#x}\nmap_extra:\t0x0\nmemlock:\t0\nmap_id:\t{}\nfrozen:\t{}\n",
            self.map_type, self.key_size, self.value_size, self.max_entries, self.flags, self.id, frozen
        )
    }
}

/// A loaded program: kept, never run.
#[derive(Debug)]
pub struct Prog {
    pub id: u32,
    pub prog_type: u32,
    pub insn_cnt: u32,
    pub tag: [u8; 8],
}

/// Why the verifier refused a program.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The error `bpf(2)` returns.
    Error(Errno),
    /// `EACCES`, with the verifier's log.
    Rejected(String),
}

const BPF_EXIT: u8 = 0x95;
const BPF_CALL: u8 = 0x85;
const BPF_MOV64_IMM: u8 = 0xb7;
const CLASS_ALU: u8 = 4;
const CLASS_ALU64: u8 = 7;

impl Prog {
    /// `BPF_PROG_LOAD` of `insns` (8 bytes each): `E2BIG` for no or too
    /// many instructions, `EINVAL` for an unknown type, one Poly cannot
    /// run, or a program that does not end in `exit`; `EACCES` (with a
    /// log) for one the verifier rejects.
    pub fn load(prog_type: u32, insns: &[u8]) -> Result<Arc<Prog>, Refused> {
        let count = insns.len() / 8;
        if count == 0 || count > INSN_MAX {
            return Err(Refused::Error(Errno::E2BIG));
        }
        if prog_type != PROG_TYPE_SOCKET_FILTER {
            return Err(Refused::Error(Errno::EINVAL));
        }
        if insns[(count - 1) * 8] != BPF_EXIT {
            return Err(Refused::Error(Errno::EINVAL));
        }
        verify(insns)?;
        let mut tag = [0u8; 8];
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in insns {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
        tag.copy_from_slice(&h.to_be_bytes());
        Ok(Arc::new(Prog { id: next_id(), prog_type, insn_cnt: count as u32, tag }))
    }

    /// `struct bpf_prog_info`: type, id, tag and the instruction count.
    pub fn info(&self) -> [u8; PROG_INFO_SIZE] {
        let mut b = [0u8; PROG_INFO_SIZE];
        b[0..4].copy_from_slice(&self.prog_type.to_le_bytes());
        b[4..8].copy_from_slice(&self.id.to_le_bytes());
        b[8..16].copy_from_slice(&self.tag);
        b[20..24].copy_from_slice(&(self.insn_cnt * 8).to_le_bytes());
        b
    }

    /// The program's lines of `/proc/<pid>/fdinfo/<fd>`.
    pub fn fdinfo(&self) -> String {
        let tag: String = self.tag.iter().map(|b| format!("{b:02x}")).collect();
        format!(
            "prog_type:\t{}\nprog_jited:\t0\nprog_tag:\t{}\nmemlock:\t0\nprog_id:\t{}\n",
            self.prog_type, tag, self.id
        )
    }
}

/// A tiny verifier: straight-line register initialisation. `r1` (the
/// context) and `r10` (the frame) start set; a move sets its target;
/// arithmetic and `exit` read registers that must be set. Jumps and
/// memory instructions are taken as they come, for a program that is
/// never run.
fn verify(insns: &[u8]) -> Result<(), Refused> {
    let mut set: u16 = 1 << 1 | 1 << 10;
    let mut log = String::new();
    for (pc, insn) in insns.chunks_exact(8).enumerate() {
        let (code, regs) = (insn[0], insn[1]);
        let (dst, src) = (regs & 0xf, regs >> 4);
        let reads = |r: u8, set: u16| set & (1 << r) != 0;
        let reject = |log: &mut String, r: u8| {
            log.push_str(&format!("{pc}: (0x{code:02x}) \nR{r} !read_ok\n"));
        };
        if code == BPF_EXIT {
            if !reads(0, set) {
                reject(&mut log, 0);
                return Err(Refused::Rejected(log));
            }
        } else if code == BPF_CALL {
            set |= 1;
        } else if matches!(code & 7, CLASS_ALU | CLASS_ALU64) {
            let op = code & 0xf0;
            let uses_reg = code & 8 != 0;
            if uses_reg && !reads(src, set) {
                reject(&mut log, src);
                return Err(Refused::Rejected(log));
            }
            // MOV (0xb0) only writes; the others read their target too.
            if op != 0xb0 && !reads(dst, set) {
                reject(&mut log, dst);
                return Err(Refused::Rejected(log));
            }
            set |= 1 << dst;
        } else if code == BPF_MOV64_IMM {
            set |= 1 << dst;
        } else if dst == 0 {
            set |= 1;
        }
    }
    Ok(())
}

/// What a descriptor of `bpf(2)` is open on.
#[derive(Clone, Debug)]
pub enum Object {
    Map(Arc<Map>),
    Prog(Arc<Prog>),
}

impl Object {
    /// What follows `anon_inode:` in the descriptor's `/proc/<pid>/fd` link.
    pub fn anon_name(&self) -> &'static str {
        match self {
            Object::Map(_) => "bpf-map",
            Object::Prog(_) => "bpf-prog",
        }
    }

    pub fn fdinfo(&self) -> String {
        match self {
            Object::Map(m) => m.fdinfo(),
            Object::Prog(p) => p.fdinfo(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(map_type: u32, key: u32, value: u32, max: u32, flags: u32) -> MapAttr {
        MapAttr { map_type, key_size: key, value_size: value, max_entries: max, map_flags: flags, ..MapAttr::default() }
    }

    #[test]
    fn map_attribute_rules() {
        let bad = |a: MapAttr| Map::create(&a).err();
        assert_eq!(bad(attr(0, 4, 8, 4, 0)), Some(Errno::EINVAL));
        assert_eq!(bad(attr(9999, 4, 8, 4, 0)), Some(Errno::EINVAL));
        assert_eq!(bad(attr(MAP_TYPE_ARRAY, 8, 8, 4, 0)), Some(Errno::EINVAL));
        assert_eq!(bad(attr(MAP_TYPE_ARRAY, 4, 0, 4, 0)), Some(Errno::EINVAL));
        assert_eq!(bad(attr(MAP_TYPE_ARRAY, 4, 8, 0, 0)), Some(Errno::EINVAL));
        assert_eq!(bad(attr(MAP_TYPE_HASH, 0, 8, 4, 0)), Some(Errno::EINVAL));
        assert_eq!(bad(attr(MAP_TYPE_ARRAY, 4, 8, 4, 1 << 31)), Some(Errno::EINVAL));
        assert_eq!(bad(attr(MAP_TYPE_ARRAY, 4, 8, 4, flag::NO_PREALLOC)), Some(Errno::EINVAL));
        assert_eq!(bad(attr(MAP_TYPE_HASH, 4, 8, 4, flag::RDONLY | flag::WRONLY)), Some(Errno::EINVAL));
        let mut named = attr(MAP_TYPE_HASH, 4, 8, 4, 0);
        named.name[0] = b'!';
        assert_eq!(bad(named), Some(Errno::EINVAL));
        named.name = [b'a'; NAME_LEN];
        assert_eq!(bad(named), Some(Errno::EINVAL), "no NUL");
        named.name[15] = 0;
        assert!(Map::create(&named).is_ok());
        for f in [flag::RDONLY, flag::RDONLY_PROG, flag::MMAPABLE] {
            assert!(Map::create(&attr(MAP_TYPE_ARRAY, 4, 8, 4, f)).is_ok());
        }
        assert!(Map::create(&attr(MAP_TYPE_HASH, 600, 1 << 20, 4, flag::NO_PREALLOC)).is_ok());
    }

    #[test]
    fn array_elements() {
        let m = Map::create(&attr(MAP_TYPE_ARRAY, 4, 8, 4, 0)).unwrap();
        let k = |i: u32| i.to_le_bytes();
        assert_eq!(m.lookup(&k(0)).unwrap(), [0; 8]);
        m.update(&k(0), &[1; 8], ANY).unwrap();
        assert_eq!(m.lookup(&k(0)).unwrap(), [1; 8]);
        assert_eq!(m.update(&k(0), &[2; 8], NOEXIST), Err(Errno::EEXIST));
        m.update(&k(0), &[3; 8], EXIST).unwrap();
        assert_eq!(m.update(&k(4), &[0; 8], ANY), Err(Errno::E2BIG));
        assert_eq!(m.lookup(&k(4)), Err(Errno::ENOENT));
        for f in [3, 99, 4] {
            assert_eq!(m.update(&k(1), &[0; 8], f), Err(Errno::EINVAL));
        }
        assert_eq!(m.delete(&k(1)), Err(Errno::EINVAL));
        assert_eq!(m.next_key(Some(&k(0))).unwrap(), k(1));
        assert_eq!(m.next_key(Some(&k(2))).unwrap(), k(3));
        assert_eq!(m.next_key(Some(&k(3))), Err(Errno::ENOENT));
        assert_eq!(m.next_key(Some(&k(100))).unwrap(), k(0));
        assert_eq!(m.next_key(None).unwrap(), k(0));
    }

    #[test]
    fn hash_elements() {
        let m = Map::create(&attr(MAP_TYPE_HASH, 2, 3, 2, 0)).unwrap();
        assert_eq!(m.lookup(&[1, 1]), Err(Errno::ENOENT));
        assert_eq!(m.delete(&[1, 1]), Err(Errno::ENOENT));
        assert_eq!(m.update(&[1, 1], &[9; 3], EXIST), Err(Errno::ENOENT));
        m.update(&[1, 1], &[9; 3], NOEXIST).unwrap();
        assert_eq!(m.update(&[1, 1], &[8; 3], NOEXIST), Err(Errno::EEXIST));
        m.update(&[2, 2], &[7; 3], ANY).unwrap();
        assert_eq!(m.update(&[3, 3], &[7; 3], ANY), Err(Errno::E2BIG), "full");
        m.update(&[2, 2], &[6; 3], ANY).unwrap();
        assert_eq!(m.lookup(&[2, 2]).unwrap(), [6; 3]);
        // Every key once, then ENOENT.
        let first = m.next_key(None).unwrap();
        let second = m.next_key(Some(&first)).unwrap();
        assert_ne!(first, second);
        assert_eq!(m.next_key(Some(&second)), Err(Errno::ENOENT));
        assert_eq!(m.next_key(Some(&[9, 9])).unwrap(), first, "a key not in the map");
        m.delete(&[1, 1]).unwrap();
        m.update(&[3, 3], &[1; 3], ANY).unwrap();
    }

    #[test]
    fn frozen_maps_and_access_flags() {
        let m = Map::create(&attr(MAP_TYPE_HASH, 1, 1, 4, 0)).unwrap();
        m.update(&[1], &[1], ANY).unwrap();
        m.freeze().unwrap();
        assert_eq!(m.update(&[1], &[2], ANY), Err(Errno::EPERM));
        assert_eq!(m.delete(&[1]), Err(Errno::EPERM));
        assert_eq!(m.lookup(&[1]).unwrap(), [1]);
        assert_eq!(m.freeze(), Err(Errno::EPERM));
        let ro = Map::create(&attr(MAP_TYPE_HASH, 1, 1, 4, flag::RDONLY)).unwrap();
        assert_eq!(ro.update(&[1], &[2], ANY), Err(Errno::EPERM));
        let wo = Map::create(&attr(MAP_TYPE_HASH, 1, 1, 4, flag::WRONLY)).unwrap();
        wo.update(&[1], &[2], ANY).unwrap();
        assert_eq!(wo.lookup(&[1]), Err(Errno::EPERM));
    }

    #[test]
    fn info_and_fdinfo() {
        let m = Map::create(&attr(MAP_TYPE_HASH, 4, 8, 2, 0)).unwrap();
        let i = m.info();
        assert_eq!((u32_at(&i, 0), u32_at(&i, 8), u32_at(&i, 12), u32_at(&i, 16)), (1, 4, 8, 2));
        assert_ne!(u32_at(&i, 4), 0);
        let other = Map::create(&attr(MAP_TYPE_HASH, 4, 8, 2, 0)).unwrap();
        assert_ne!(other.id, m.id);
        let text = m.fdinfo();
        for line in ["map_type:\t1\n", "key_size:\t4\n", "value_size:\t8\n", "max_entries:\t2\n", "frozen:\t0\n"] {
            assert!(text.contains(line), "{line}");
        }
    }

    fn insn(code: u8, regs: u8, imm: i32) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0] = code;
        b[1] = regs;
        b[4..8].copy_from_slice(&imm.to_le_bytes());
        b
    }

    fn prog(list: &[[u8; 8]]) -> Vec<u8> {
        list.concat()
    }

    #[test]
    fn programs_load_and_are_verified() {
        let mov = insn(BPF_MOV64_IMM, 0, 0);
        let exit = insn(BPF_EXIT, 0, 0);
        let ok = Prog::load(1, &prog(&[mov, exit])).unwrap();
        assert_eq!((ok.insn_cnt, ok.prog_type), (2, 1));
        assert!(ok.fdinfo().contains("prog_type:\t1\n"));
        assert_eq!(Prog::load(1, &[]).err(), Some(Refused::Error(Errno::E2BIG)));
        assert_eq!(Prog::load(1, &prog(&[mov])).err(), Some(Refused::Error(Errno::EINVAL)), "no exit");
        assert_eq!(Prog::load(9999, &prog(&[mov, exit])).err(), Some(Refused::Error(Errno::EINVAL)));
        match Prog::load(1, &prog(&[exit])) {
            Err(Refused::Rejected(log)) => assert!(log.contains("R0 !read_ok")),
            other => panic!("{other:?}"),
        }
        // r2 = r3 reads an unset register.
        let bad = insn(0xbf, 0x32, 0);
        assert!(matches!(Prog::load(1, &prog(&[bad, mov, exit])), Err(Refused::Rejected(_))));
        // r0 = r1 is fine: r1 is the context.
        let ok = insn(0xbf, 0x10, 0);
        assert!(Prog::load(1, &prog(&[ok, exit])).is_ok());
    }

    #[test]
    fn names() {
        let mut n = [0u8; NAME_LEN];
        assert!(name_ok(&n));
        n[..4].copy_from_slice(b"a.b_");
        assert!(name_ok(&n));
        n[1] = b'-';
        assert!(!name_ok(&n));
    }
}
