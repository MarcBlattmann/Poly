//! The global atom table (`GlobalAddAtomW`, `GlobalFindAtomW`, `GlobalDeleteAtom`,
//! `GlobalGetAtomNameW`; Learn: "About Atom Tables"): strings of at most 255 characters mapped to
//! numbers in 0xC000..=0xFFFF, compared without regard to case, counted by how often they were
//! added. Integer atoms (1..=0xBFFF, `MAKEINTATOM`) are no table entries: they stand for
//! themselves, and the string `#1234` names the integer atom 1234.

use alloc::vec::Vec;

/// Atoms made by strings start here (`winuser.h`'s `MAXINTATOM` is 0xC000).
pub const FIRST_STRING_ATOM: u16 = 0xC000;
/// The longest atom name.
pub const MAX_NAME: usize = 255;

// winerror.h
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_MORE_DATA: u32 = 234;

#[derive(Debug)]
struct Entry {
    atom: u16,
    name: Vec<u16>,
    refs: u32,
}

/// One table.
#[derive(Debug, Default)]
pub struct Atoms {
    list: Vec<Entry>,
}

fn fold(c: u16) -> u16 {
    match char::from_u32(u32::from(c)) {
        Some(ch) => {
            let mut l = ch.to_lowercase();
            match (l.next(), l.next()) {
                (Some(a), None) if (a as u32) < 0x10000 => a as u32 as u16,
                _ => c,
            }
        }
        None => c,
    }
}

fn same(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| fold(x) == fold(y))
}

/// `#1234`: the integer atom a name stands for (1..=0xBFFF), else `None`.
pub fn integer_atom(name: &[u16]) -> Option<u16> {
    let digits = name.strip_prefix(&[u16::from(b'#')])?;
    if digits.is_empty() || !digits.iter().all(|&c| (u16::from(b'0')..=u16::from(b'9')).contains(&c)) {
        return None;
    }
    let mut n: u32 = 0;
    for &c in digits {
        n = n.checked_mul(10)?.checked_add(u32::from(c - u16::from(b'0')))?;
    }
    (1..FIRST_STRING_ATOM as u32).contains(&n).then_some(n as u16)
}

impl Atoms {
    pub const fn new() -> Atoms {
        Atoms { list: Vec::new() }
    }

    /// `GlobalAddAtomW(name)`: the atom (counted once more), or the last error.
    pub fn add(&mut self, name: &[u16]) -> Result<u16, u32> {
        if name.is_empty() || name.len() > MAX_NAME {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if let Some(a) = integer_atom(name) {
            return Ok(a);
        }
        if let Some(e) = self.list.iter_mut().find(|e| same(&e.name, name)) {
            e.refs += 1;
            return Ok(e.atom);
        }
        let used = |a: u16| self.list.iter().any(|e| e.atom == a);
        let atom = (FIRST_STRING_ATOM..=u16::MAX).find(|&a| !used(a)).ok_or(ERROR_INVALID_PARAMETER)?;
        self.list.push(Entry { atom, name: name.to_vec(), refs: 1 });
        Ok(atom)
    }

    /// `GlobalFindAtomW(name)`: the atom, not counted.
    pub fn find(&self, name: &[u16]) -> Result<u16, u32> {
        if name.is_empty() || name.len() > MAX_NAME {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if let Some(a) = integer_atom(name) {
            return Ok(a);
        }
        self.list.iter().find(|e| same(&e.name, name)).map(|e| e.atom).ok_or(ERROR_FILE_NOT_FOUND)
    }

    /// `GlobalDeleteAtom(atom)`: one count less, the entry goes with the last. Integer atoms are
    /// never deleted (Learn: "returns zero", they do nothing).
    pub fn delete(&mut self, atom: u16) -> Result<(), u32> {
        if atom < FIRST_STRING_ATOM {
            return if atom == 0 { Err(ERROR_INVALID_HANDLE) } else { Ok(()) };
        }
        let Some(i) = self.list.iter().position(|e| e.atom == atom) else { return Err(ERROR_INVALID_HANDLE) };
        self.list[i].refs -= 1;
        if self.list[i].refs == 0 {
            self.list.remove(i);
        }
        Ok(())
    }

    /// The name of a string atom (as it was first written), or `#n` for an integer atom.
    pub fn name(&self, atom: u16) -> Option<Vec<u16>> {
        if atom == 0 {
            return None;
        }
        if atom < FIRST_STRING_ATOM {
            return Some(alloc::format!("#{atom}").encode_utf16().collect());
        }
        self.list.iter().find(|e| e.atom == atom).map(|e| e.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn strings_are_counted_and_compared_without_case() {
        let mut t = Atoms::new();
        let a = t.add(&w("ByAtom")).unwrap();
        assert!(a >= FIRST_STRING_ATOM);
        assert_eq!(t.add(&w("BYATOM")), Ok(a));
        assert_eq!(t.find(&w("byatom")), Ok(a));
        assert_eq!(t.name(a), Some(w("ByAtom")));
        assert_eq!(t.delete(a), Ok(()));
        assert_eq!(t.find(&w("ByAtom")), Ok(a));
        assert_eq!(t.delete(a), Ok(()));
        assert_eq!(t.find(&w("ByAtom")), Err(ERROR_FILE_NOT_FOUND));
        assert_eq!(t.delete(a), Err(ERROR_INVALID_HANDLE));
    }

    #[test]
    fn integer_atoms_and_bad_names() {
        let mut t = Atoms::new();
        assert_eq!(t.add(&w("#1234")), Ok(1234));
        assert_eq!(t.name(1234), Some(w("#1234")));
        assert_eq!(t.add(&[]), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(t.add(&w(&"x".repeat(256))), Err(ERROR_INVALID_PARAMETER));
        assert!(t.add(&w(&"x".repeat(255))).is_ok());
        assert!(t.add(&w("#49152")).is_ok(), "above the integer range: an ordinary string");
        assert_eq!(t.delete(5), Ok(()));
    }
}
