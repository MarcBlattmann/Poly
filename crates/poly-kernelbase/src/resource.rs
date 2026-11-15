//! The PE resource API of `kernelbase.dll` and `user32.dll` (`FindResourceW`, `FindResourceExW`,
//! `LoadResource`, `LockResource`, `SizeofResource`, `EnumResourceNamesW`, `EnumResourceTypesW`,
//! `LoadStringW`) as host-tested logic: what a name or type argument means, how the resource
//! directory is searched and listed, how a string table block is read.
//!
//! Sources: the PE format specification, "The .rsrc Section" (the directory tables: a type level,
//! a name level and a language level, entries by name or by id, the high bit of an offset marking
//! a subdirectory), Learn's pages for the functions ("MAKEINTRESOURCE", "Resource Types", the
//! string table resource), and the spec sheets of `specs/windows/kernel32.dll/FindResourceW.yaml`
//! and the others of the batch (observed on Windows): names and types are matched without regard
//! to case against the upper-case stored names, `#number` strings, the language rules and the
//! error codes.

use alloc::vec::Vec;
use poly_ntobj::name::upcase;

/// `ERROR_INVALID_PARAMETER`, `ERROR_RESOURCE_DATA_NOT_FOUND`, `ERROR_RESOURCE_TYPE_NOT_FOUND`,
/// `ERROR_RESOURCE_NAME_NOT_FOUND`, `ERROR_RESOURCE_LANG_NOT_FOUND`,
/// `ERROR_RESOURCE_ENUM_USER_STOP` (`winerror.h`).
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_RESOURCE_DATA_NOT_FOUND: u32 = 1812;
pub const ERROR_RESOURCE_TYPE_NOT_FOUND: u32 = 1813;
pub const ERROR_RESOURCE_NAME_NOT_FOUND: u32 = 1814;
pub const ERROR_RESOURCE_LANG_NOT_FOUND: u32 = 1815;
pub const ERROR_RESOURCE_ENUM_USER_STOP: u32 = 15106;

/// `RT_STRING` (`winuser.h`).
pub const RT_STRING: u32 = 6;

/// The language `LANG_NEUTRAL` and en-US (`winnt.h`: `MAKELANGID`).
pub const LANG_NEUTRAL: u16 = 0;
pub const LANG_EN_US: u16 = 0x0409;

/// A type or name as a caller gave it: an integer id, or a string (upper-cased: the stored names
/// are upper case, and a lookup matches without regard to case).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Id {
    Num(u32),
    Name(Vec<u16>),
}

fn upper(s: &[u16]) -> Vec<u16> {
    s.iter().map(|&c| upcase(c)).collect()
}

/// What a string argument stands for: a name, or with a leading `#` the integer after it
/// (`specs/windows/kernel32.dll/FindResourceW.yaml`, `number_form`): white space after the `#`
/// is skipped, the digits are read up to the first character that is not one (no digits: the id
/// 0); `#` alone and numbers above 65535 are `ERROR_INVALID_PARAMETER`.
pub fn parse_string_id(s: &[u16]) -> Result<Id, u32> {
    if s.first() != Some(&u16::from(b'#')) {
        return Ok(Id::Name(upper(s)));
    }
    let rest = &s[1..];
    if rest.is_empty() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let rest = &rest[rest.iter().position(|&c| c > 0x20).unwrap_or(rest.len())..];
    let mut n: u32 = 0;
    for &c in rest.iter().take_while(|&&c| (u16::from(b'0')..=u16::from(b'9')).contains(&c)) {
        n = n * 10 + u32::from(c - u16::from(b'0'));
        if n > 0xFFFF {
            return Err(ERROR_INVALID_PARAMETER);
        }
    }
    Ok(Id::Num(n))
}

/// The size of an `IMAGE_RESOURCE_DIRECTORY` and of one entry; of an `IMAGE_RESOURCE_DATA_ENTRY`.
const DIR: usize = 16;
const ENTRY: usize = 8;
/// A resource tree (the bytes of the resource directory of a module, from its first directory
/// table); every offset is checked.
#[derive(Clone, Copy, Debug)]
pub struct Tree<'a>(pub &'a [u8]);

const HIGH: u32 = 0x8000_0000;
/// `sizeof(IMAGE_RESOURCE_DATA_ENTRY)`.
pub const DATA_ENTRY: usize = 16;

/// A directory table: its offset in the tree and the entries `(name_or_id, offset)`.
struct Dir {
    named: usize,
    entries: Vec<(u32, u32)>,
}

impl<'a> Tree<'a> {
    fn u32_at(&self, at: usize) -> Option<u32> {
        self.0.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        self.0.get(at..at + 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()))
    }

    fn dir(&self, at: usize) -> Option<Dir> {
        let named = usize::from(self.u16_at(at + 12)?);
        let ids = usize::from(self.u16_at(at + 14)?);
        let mut entries = Vec::with_capacity(named + ids);
        for i in 0..named + ids {
            let e = at + DIR + i * ENTRY;
            entries.push((self.u32_at(e)?, self.u32_at(e + 4)?));
        }
        Some(Dir { named, entries })
    }

    /// The string a name entry points at (a length in units, then the units).
    fn name_at(&self, offset: u32) -> Option<Vec<u16>> {
        let at = (offset & !HIGH) as usize;
        let len = usize::from(self.u16_at(at)?);
        let bytes = self.0.get(at + 2..at + 2 + len * 2)?;
        Some(bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
    }

    /// The entries of a directory as `(key, offset)` in the order named ascending, then ids ascending.
    fn listing(&self, at: usize) -> Option<Vec<(Id, u32)>> {
        let d = self.dir(at)?;
        let mut named: Vec<(Id, u32)> = Vec::new();
        let mut ids: Vec<(Id, u32)> = Vec::new();
        for (i, &(key, off)) in d.entries.iter().enumerate() {
            if i < d.named && key & HIGH != 0 {
                named.push((Id::Name(self.name_at(key)?), off));
            } else {
                ids.push((Id::Num(key & 0xFFFF), off));
            }
        }
        named.sort_by(|a, b| match (&a.0, &b.0) {
            (Id::Name(x), Id::Name(y)) => upper(x).cmp(&upper(y)),
            _ => core::cmp::Ordering::Equal,
        });
        ids.sort_by_key(|(id, _)| match id {
            Id::Num(n) => *n,
            Id::Name(_) => 0,
        });
        named.extend(ids);
        Some(named)
    }

    /// The subdirectory of `key` in the directory at `at`.
    fn child(&self, at: usize, key: &Id) -> Option<Option<u32>> {
        let list = self.listing(at)?;
        Some(list.into_iter().find(|(k, _)| same_key(k, key)).map(|(_, off)| off))
    }

    /// The names (or ids) of the resources of a type, in the order `EnumResourceNamesW` reports.
    /// `Err` is the Win32 error: `ERROR_RESOURCE_TYPE_NOT_FOUND`, or `ERROR_RESOURCE_DATA_NOT_FOUND`
    /// for a tree that cannot be read.
    pub fn names_of(&self, ty: &Id) -> Result<Vec<Id>, u32> {
        let off = self.child(0, ty).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?.ok_or(ERROR_RESOURCE_TYPE_NOT_FOUND)?;
        let list = self.listing((off & !HIGH) as usize).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
        Ok(list.into_iter().map(|(k, _)| k).collect())
    }

    /// The types of the module, named ones first.
    pub fn types(&self) -> Result<Vec<Id>, u32> {
        let list = self.listing(0).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
        Ok(list.into_iter().map(|(k, _)| k).collect())
    }

    /// `FindResourceExW`: the offset in the tree of the `IMAGE_RESOURCE_DATA_ENTRY` of the
    /// resource, or the Win32 error.
    ///
    /// The language must match exactly, except that a resource stored in the neutral language
    /// is found with any; the neutral language asked for finds the neutral resource, else the
    /// en-US one, else the first (the user interface language decides on Windows).
    pub fn find(&self, ty: &Id, name: &Id, lang: u16) -> Result<usize, u32> {
        let ty_dir = self.child(0, ty).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?.ok_or(ERROR_RESOURCE_TYPE_NOT_FOUND)?;
        let name_dir = self
            .child((ty_dir & !HIGH) as usize, name)
            .ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?
            .ok_or(ERROR_RESOURCE_NAME_NOT_FOUND)?;
        let langs = self.listing((name_dir & !HIGH) as usize).ok_or(ERROR_RESOURCE_DATA_NOT_FOUND)?;
        let pick = |want: u16| langs.iter().find(|(k, _)| *k == Id::Num(u32::from(want)));
        let chosen = if lang == LANG_NEUTRAL {
            pick(LANG_NEUTRAL).or_else(|| pick(LANG_EN_US)).or_else(|| langs.first())
        } else {
            pick(lang).or_else(|| pick(LANG_NEUTRAL))
        };
        let (_, off) = chosen.ok_or(ERROR_RESOURCE_LANG_NOT_FOUND)?;
        // A leaf: the offset is that of a data entry (the high bit is clear).
        if off & HIGH != 0 {
            return Err(ERROR_RESOURCE_DATA_NOT_FOUND);
        }
        let at = *off as usize;
        if at + DATA_ENTRY > self.0.len() {
            return Err(ERROR_RESOURCE_DATA_NOT_FOUND);
        }
        Ok(at)
    }

    /// `(rva, size)` of the data entry at `at` (an offset [`Tree::find`] returned).
    pub fn data(&self, at: usize) -> Option<(u32, u32)> {
        Some((self.u32_at(at)?, self.u32_at(at + 4)?))
    }
}

fn same_key(a: &Id, b: &Id) -> bool {
    match (a, b) {
        (Id::Num(x), Id::Num(y)) => x == y,
        (Id::Name(x), Id::Name(y)) => upper(x) == upper(y),
        _ => false,
    }
}

/// Where the strings of a string-table block start: `(offset in the block in bytes, length in
/// units)` of the string with index `index` (0 to 15). The block holds 16 strings, each a
/// length in units then the units, without a NUL. `None` when the block is too short.
pub fn string_in_block(block: &[u8], index: usize) -> Option<(usize, usize)> {
    let mut at = 0;
    for i in 0..16 {
        let len = usize::from(u16::from_le_bytes(block.get(at..at + 2)?.try_into().unwrap()));
        if i == index {
            if at + 2 + len * 2 > block.len() {
                return None;
            }
            return Some((at + 2, len));
        }
        at += 2 + len * 2;
    }
    None
}

/// `LoadStringW`'s block id and index for a string id: the string `id` is entry `id & 15` of the
/// `RT_STRING` block `(id >> 4) + 1`, with the id taken as a 16-bit value (an id above 65535
/// names the block of its low 16 bits, as observed).
pub fn string_block_and_index(id: u32) -> (u32, usize) {
    (u32::from(((id & 0xFFFF) >> 4) as u16) + 1, (id & 15) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn tree() -> Vec<u8> {
        // type TEXT (named) -> names "B", "A" (named, stored out of order) and 7 (id);
        // type 10 -> name 100 -> languages 0x409 and 0x407; type 6 -> name 7 -> lang 0x407.
        // Built by hand: the offsets are computed first.
        let mut t: Vec<u8> = Vec::new();
        let put32 = |t: &mut Vec<u8>, v: u32| t.extend_from_slice(&v.to_le_bytes());
        let put16 = |t: &mut Vec<u8>, v: u16| t.extend_from_slice(&v.to_le_bytes());
        // Layout (offsets): 0 root(2 entries: named TEXT, id 10 ... plus id 6)
        // We keep it simple: root has 1 named entry (TEXT) and 2 ids (6, 10).
        let root = 0usize;
        let root_len = DIR + 3 * ENTRY;
        let text_dir = root + root_len; // 3 entries: named A, named B, id 7
        let text_len = DIR + 3 * ENTRY;
        let ten_dir = text_dir + text_len; // 1 entry: id 100
        let ten_len = DIR + ENTRY;
        let ten100_dir = ten_dir + ten_len; // 2 entries: 0x407, 0x409
        let ten100_len = DIR + 2 * ENTRY;
        let six_dir = ten100_dir + ten100_len; // 1 entry: id 7
        let six_len = DIR + ENTRY;
        let six7_dir = six_dir + six_len; // 1 entry: 0x407
        let six7_len = DIR + ENTRY;
        let a_dir = six7_dir + six7_len; // 1 entry: lang 0
        let a_len = DIR + ENTRY;
        let b_dir = a_dir + a_len;
        let b_len = DIR + ENTRY;
        let seven_dir = b_dir + b_len;
        let seven_len = DIR + ENTRY;
        let data0 = seven_dir + seven_len; // data entries, 16 bytes each: A, B, 7, 10/100/407, 10/100/409, 6/7/407
        let name_text = data0 + 6 * DATA_ENTRY;
        let name_a = name_text + 2 + 4 * 2; // "TEXT": a length and 4 units
        let name_b = name_a + 2 + 2;
        let dir_header = |t: &mut Vec<u8>, named: u16, ids: u16| {
            t.extend_from_slice(&[0; 12]);
            put16(t, named);
            put16(t, ids);
        };
        // root
        dir_header(&mut t, 1, 2);
        put32(&mut t, HIGH | name_text as u32);
        put32(&mut t, HIGH | text_dir as u32);
        put32(&mut t, 6);
        put32(&mut t, HIGH | six_dir as u32);
        put32(&mut t, 10);
        put32(&mut t, HIGH | ten_dir as u32);
        // TEXT: B before A in the file on purpose, then id 7.
        dir_header(&mut t, 2, 1);
        put32(&mut t, HIGH | name_b as u32);
        put32(&mut t, HIGH | b_dir as u32);
        put32(&mut t, HIGH | name_a as u32);
        put32(&mut t, HIGH | a_dir as u32);
        put32(&mut t, 7);
        put32(&mut t, HIGH | seven_dir as u32);
        // type 10
        dir_header(&mut t, 0, 1);
        put32(&mut t, 100);
        put32(&mut t, HIGH | ten100_dir as u32);
        dir_header(&mut t, 0, 2);
        put32(&mut t, 0x407);
        put32(&mut t, (data0 + 3 * DATA_ENTRY) as u32);
        put32(&mut t, 0x409);
        put32(&mut t, (data0 + 4 * DATA_ENTRY) as u32);
        // type 6
        dir_header(&mut t, 0, 1);
        put32(&mut t, 7);
        put32(&mut t, HIGH | six7_dir as u32);
        dir_header(&mut t, 0, 1);
        put32(&mut t, 0x407);
        put32(&mut t, (data0 + 5 * DATA_ENTRY) as u32);
        // A, B, 7: neutral language leaves.
        for i in 0..3usize {
            dir_header(&mut t, 0, 1);
            put32(&mut t, 0);
            put32(&mut t, (data0 + i * DATA_ENTRY) as u32);
        }
        // Data entries: rva = 0x1000 * (n + 1), size = 10 * (n + 1).
        for n in 0..6u32 {
            put32(&mut t, 0x1000 * (n + 1));
            put32(&mut t, 10 * (n + 1));
            put32(&mut t, 0);
            put32(&mut t, 0);
        }
        // Names.
        put16(&mut t, 4);
        for c in "TEXT".encode_utf16() {
            put16(&mut t, c);
        }
        put16(&mut t, 1);
        put16(&mut t, u16::from(b'A'));
        put16(&mut t, 1);
        put16(&mut t, u16::from(b'B'));
        assert_eq!(t.len(), name_b + 4);
        t
    }

    #[test]
    fn ids_follow_the_number_form() {
        assert_eq!(parse_string_id(&w("blob")), Ok(Id::Name(w("BLOB"))));
        assert_eq!(parse_string_id(&w("#10")), Ok(Id::Num(10)));
        assert_eq!(parse_string_id(&w("#010")), Ok(Id::Num(10)));
        assert_eq!(parse_string_id(&w("#100x")), Ok(Id::Num(100)));
        assert_eq!(parse_string_id(&w("# 100")), Ok(Id::Num(100)));
        assert_eq!(parse_string_id(&w("#abc")), Ok(Id::Num(0)));
        assert_eq!(parse_string_id(&w("#")), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(parse_string_id(&w("#65636")), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(parse_string_id(&w("#65535")), Ok(Id::Num(65535)));
    }

    #[test]
    fn the_tree_is_searched_by_type_name_and_language() {
        let t = tree();
        let tree = Tree(&t);
        let ten = Id::Num(10);
        let hundred = Id::Num(100);
        // Exact language.
        let at = tree.find(&ten, &hundred, 0x409).unwrap();
        assert_eq!(tree.data(at), Some((0x5000, 50)));
        let at = tree.find(&ten, &hundred, 0x407).unwrap();
        assert_eq!(tree.data(at), Some((0x4000, 40)));
        // No other language, and the neutral one finds en-US first.
        assert_eq!(tree.find(&ten, &hundred, 0x809), Err(ERROR_RESOURCE_LANG_NOT_FOUND));
        assert_eq!(tree.find(&ten, &hundred, 0x0001), Err(ERROR_RESOURCE_LANG_NOT_FOUND));
        let at = tree.find(&ten, &hundred, 0).unwrap();
        assert_eq!(tree.data(at), Some((0x5000, 50)));
        // A neutral resource is found with any language, names without regard to case.
        let text = Id::Name(w("TEXT"));
        let a = tree.find(&text, &Id::Name(upper(&w("a"))), 0x411).unwrap();
        assert_eq!(tree.data(a), Some((0x1000, 10)));
        // Only one language: found with it and with the neutral one.
        let six = Id::Num(6);
        assert!(tree.find(&six, &Id::Num(7), 0x407).is_ok());
        assert!(tree.find(&six, &Id::Num(7), 0).is_ok());
        assert_eq!(tree.find(&six, &Id::Num(7), 0x409), Err(ERROR_RESOURCE_LANG_NOT_FOUND));
        // Errors.
        assert_eq!(tree.find(&Id::Num(99), &hundred, 0), Err(ERROR_RESOURCE_TYPE_NOT_FOUND));
        assert_eq!(tree.find(&ten, &Id::Num(1), 0), Err(ERROR_RESOURCE_NAME_NOT_FOUND));
        assert_eq!(tree.find(&ten, &Id::Name(w("BLOB")), 0), Err(ERROR_RESOURCE_NAME_NOT_FOUND));
    }

    #[test]
    fn listings_put_names_first_in_ascending_order() {
        let t = tree();
        let tree = Tree(&t);
        assert_eq!(tree.names_of(&Id::Name(w("TEXT"))).unwrap(), [Id::Name(w("A")), Id::Name(w("B")), Id::Num(7)]);
        assert_eq!(tree.types().unwrap(), [Id::Name(w("TEXT")), Id::Num(6), Id::Num(10)]);
        assert_eq!(tree.names_of(&Id::Num(10)).unwrap(), [Id::Num(100)]);
        assert_eq!(tree.names_of(&Id::Num(3)), Err(ERROR_RESOURCE_TYPE_NOT_FOUND));
        // A tree that is too short cannot be read.
        assert_eq!(Tree(&t[..10]).types(), Err(ERROR_RESOURCE_DATA_NOT_FOUND));
    }

    #[test]
    fn string_blocks() {
        // Strings 0..15: "" , "Hello", "", "ab", then empty ones.
        let mut block: Vec<u8> = Vec::new();
        for s in ["", "Hello", "", "ab"] {
            block.extend_from_slice(&(s.len() as u16).to_le_bytes());
            for c in s.encode_utf16() {
                block.extend_from_slice(&c.to_le_bytes());
            }
        }
        for _ in 0..12 {
            block.extend_from_slice(&[0, 0]);
        }
        assert_eq!(string_in_block(&block, 1), Some((4, 5)));
        assert_eq!(string_in_block(&block, 0), Some((2, 0)));
        assert_eq!(string_in_block(&block, 3), Some((4 + 10 + 2 + 2, 2)));
        assert_eq!(string_in_block(&block, 15), Some((block.len(), 0)));
        assert_eq!(string_in_block(&block[..6], 1), None);
        assert_eq!(string_block_and_index(1), (1, 1));
        assert_eq!(string_block_and_index(16), (2, 0));
        assert_eq!(string_block_and_index(100), (7, 4));
        // An id above 65535 names the block of its low 16 bits.
        assert_eq!(string_block_and_index(0x10000), (1, 0));
        assert_eq!(string_block_and_index(0xFFFF_FFFF), (4096, 15));
    }
}
