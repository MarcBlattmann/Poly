//! The environment block: `Name=Value\0` strings ended by one more `\0`
//! (Learn: "Environment Variables", "Changing Environment Variables").
//!
//! Names are compared without case. A name may start with `=` (the
//! per-drive current directories, `=C:=C:\dir`), so the separating `=` is
//! the first one after the first character. The block is kept sorted by
//! name without case, in UTF-16 code-unit order ("All strings in the
//! environment block must be sorted alphabetically by name. The sort is
//! case-insensitive, Unicode order, without regard to locale").
//!
//! Choices to be confirmed by the spec agent (ADR 0079): deleting a
//! variable that does not exist succeeds; a name that is empty or has a
//! `=` after its first character is `ERROR_INVALID_PARAMETER`.

use crate::error::ERROR_INVALID_PARAMETER;
use alloc::vec::Vec;
use poly_ntobj::name::upcase;

const EQ: u16 = b'=' as u16;
const PCT: u16 = b'%' as u16;

/// The entries of a block, without their NULs, up to the empty one that
/// ends it (or the end of the slice).
pub fn entries(block: &[u16]) -> impl Iterator<Item = &[u16]> {
    block.split(|&c| c == 0).take_while(|e| !e.is_empty())
}

/// An entry's name and value (`None` for an entry without `=`).
pub fn split(entry: &[u16]) -> Option<(&[u16], &[u16])> {
    let eq = entry.iter().skip(1).position(|&c| c == EQ)? + 1;
    Some((&entry[..eq], &entry[eq + 1..]))
}

fn cmp_names(a: &[u16], b: &[u16]) -> core::cmp::Ordering {
    a.iter().map(|&c| upcase(c)).cmp(b.iter().map(|&c| upcase(c)))
}

/// The value of variable `name`.
pub fn get<'a>(block: &'a [u16], name: &[u16]) -> Option<&'a [u16]> {
    entries(block).filter_map(split).find(|(n, _)| cmp_names(n, name).is_eq()).map(|(_, v)| v)
}

/// Whether `name` may be set (`SetEnvironmentVariableW`).
pub fn valid_name(name: &[u16]) -> bool {
    !name.is_empty() && !name[1..].contains(&EQ)
}

/// The block with `name` set to `value`, or removed (`None`), sorted.
pub fn set(block: &[u16], name: &[u16], value: Option<&[u16]>) -> Result<Vec<u16>, u32> {
    if !valid_name(name) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut list: Vec<&[u16]> =
        entries(block).filter(|e| split(e).is_none_or(|(n, _)| !cmp_names(n, name).is_eq())).collect();
    let mut new = Vec::new();
    if let Some(v) = value {
        new.extend_from_slice(name);
        new.push(EQ);
        new.extend_from_slice(v);
        let at = list.partition_point(|e| {
            let n = split(e).map_or(*e, |(n, _)| n);
            cmp_names(n, name).is_lt()
        });
        list.insert(at, &new);
    }
    Ok(build(list.iter().copied()))
}

/// A block from entries (each without its NUL).
pub fn build<'a>(list: impl Iterator<Item = &'a [u16]>) -> Vec<u16> {
    let mut out = Vec::new();
    for e in list {
        out.extend_from_slice(e);
        out.push(0);
    }
    if out.is_empty() {
        out.push(0);
    }
    out.push(0);
    out
}

/// The number of units of a block at `p`, both final NULs included.
///
/// # Safety
/// `p` points at a block ended by an empty entry.
pub unsafe fn block_len(p: *const u16) -> usize {
    let mut i = 0;
    loop {
        // SAFETY: the caller's contract: we stop at the double NUL.
        let (a, b) = unsafe { (*p.add(i), *p.add(i + 1)) };
        if a == 0 && b == 0 {
            return i + 2;
        }
        i += 1;
    }
}

/// `ExpandEnvironmentStringsW`: every `%name%` whose variable is set is
/// replaced by its value; an undefined one stays as it is (Learn), and so
/// does a `%` without a closing one.
pub fn expand(src: &[u16], lookup: &dyn Fn(&[u16]) -> Option<Vec<u16>>) -> Vec<u16> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        if src[i] == PCT
            && let Some(len) = src[i + 1..].iter().position(|&c| c == PCT)
        {
            let name = &src[i + 1..i + 1 + len];
            if !name.is_empty()
                && let Some(v) = lookup(name)
            {
                out.extend_from_slice(&v);
                i += len + 2;
                continue;
            }
            // Not a variable: keep this `%` and look again from the next
            // one, which may open a real reference.
            out.extend_from_slice(&src[i..i + 1 + len]);
            i += 1 + len;
            continue;
        }
        out.push(src[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use crate::wide as w;
    use std::string::String;
    use std::vec;

    fn blk(items: &[&str]) -> Vec<u16> {
        let v: Vec<Vec<u16>> = items.iter().map(|s| w(s)).collect();
        build(v.iter().map(|e| e.as_slice()))
    }

    fn names(b: &[u16]) -> Vec<String> {
        entries(b).map(String::from_utf16_lossy).collect()
    }

    #[test]
    fn lookup_ignores_case_and_handles_drive_variables() {
        let b = blk(&["=C:=C:\\dir", "Path=C:\\Windows", "TEMP=C:\\t"]);
        assert_eq!(get(&b, &w("PATH")), Some(&w("C:\\Windows")[..]));
        assert_eq!(get(&b, &w("temp")), Some(&w("C:\\t")[..]));
        assert_eq!(get(&b, &w("=c:")), Some(&w("C:\\dir")[..]));
        assert_eq!(get(&b, &w("TMP")), None);
        assert_eq!(get(&b, &w("Pat")), None);
    }

    #[test]
    fn set_keeps_the_block_sorted() {
        let b = blk(&["A=1", "C=3"]);
        let b = set(&b, &w("b"), Some(&w("2"))).unwrap();
        assert_eq!(names(&b), vec!["A=1", "b=2", "C=3"]);
        let b = set(&b, &w("B"), Some(&w("two"))).unwrap();
        assert_eq!(names(&b), vec!["A=1", "B=two", "C=3"]);
        let b = set(&b, &w("a"), None).unwrap();
        assert_eq!(names(&b), vec!["B=two", "C=3"]);
        let b = set(&b, &w("missing"), None).unwrap();
        assert_eq!(names(&b), vec!["B=two", "C=3"]);
        let b = set(&b, &w("E"), Some(&w(""))).unwrap();
        assert_eq!(names(&b), vec!["B=two", "C=3", "E="]);
        assert_eq!(get(&b, &w("e")), Some(&[][..]));
        assert_eq!(*b.last().unwrap(), 0);
        assert_eq!(b[b.len() - 2], 0);
    }

    #[test]
    fn invalid_names() {
        let b = blk(&[]);
        assert_eq!(b, vec![0, 0]);
        assert_eq!(set(&b, &w(""), Some(&w("x"))), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set(&b, &w("A=B"), Some(&w("x"))), Err(ERROR_INVALID_PARAMETER));
        assert!(set(&b, &w("=D:"), Some(&w("D:\\"))).is_ok());
        let empty = set(&blk(&["A=1"]), &w("A"), None).unwrap();
        assert_eq!(empty, vec![0, 0]);
    }

    #[test]
    fn block_lengths() {
        let b = blk(&["A=1", "BB=2"]);
        // SAFETY: a block built above.
        assert_eq!(unsafe { block_len(b.as_ptr()) }, b.len());
        let e = blk(&[]);
        // SAFETY: as above.
        assert_eq!(unsafe { block_len(e.as_ptr()) }, 2);
    }

    #[test]
    fn expansion() {
        let b = blk(&["USERPROFILE=C:\\Users\\poly", "X=1"]);
        let look = |n: &[u16]| get(&b, n).map(|v| v.to_vec());
        let e = |s: &str| String::from_utf16_lossy(&expand(&w(s), &look));
        assert_eq!(e("%USERPROFILE%\\AppData"), "C:\\Users\\poly\\AppData");
        assert_eq!(e("%nope%\\x"), "%nope%\\x");
        assert_eq!(e("100%"), "100%");
        assert_eq!(e("%%X%"), "%1");
        assert_eq!(e("a%x%b%X%"), "a1b1");
    }
}
