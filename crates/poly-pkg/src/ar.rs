//! ar(5) archives, the outer container of a `.deb` (deb(5)): the global
//! header `!<arch>\n`, then members, each a 60-byte header (name, times,
//! owner, mode, size in decimal, the magic `` `\n ``) and its data, padded
//! to an even offset.

use crate::Error;
use alloc::string::String;
use alloc::vec::Vec;

/// The archive's first eight bytes.
pub const MAGIC: &[u8; 8] = b"!<arch>\n";
/// The length of a member's header.
pub const HEADER: usize = 60;

/// A member's header: its name and where its data is in the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The name, without the padding and without the `/` some writers end
    /// it with.
    pub name: String,
    /// The offset of the data from the start of the archive.
    pub offset: u64,
    /// The length of the data.
    pub size: u64,
}

impl Member {
    /// Where the next member's header starts.
    pub fn next(&self) -> u64 {
        self.offset + self.size + (self.size & 1)
    }
}

/// Read the member header that starts at offset `at` of the archive, given
/// its 60 bytes.
pub fn header(at: u64, bytes: &[u8]) -> Result<Member, Error> {
    let h = bytes.get(..HEADER).ok_or(Error::Truncated)?;
    if &h[58..60] != b"`\n" {
        return Err(Error::NotThisFormat);
    }
    let name = core::str::from_utf8(&h[..16]).map_err(|_| Error::NotThisFormat)?;
    let name = name.trim_end_matches(' ');
    let name = name.strip_suffix('/').unwrap_or(name);
    let size = core::str::from_utf8(&h[48..58]).ok().and_then(|s| s.trim().parse().ok()).ok_or(Error::NotThisFormat)?;
    Ok(Member { name: name.into(), offset: at + HEADER as u64, size })
}

/// The members whose headers are wholly in `data` (the archive, or its
/// beginning): the list stops where the data does.
pub fn members(data: &[u8]) -> Result<Vec<Member>, Error> {
    if data.len() < MAGIC.len() {
        return Err(Error::Truncated);
    }
    if &data[..MAGIC.len()] != MAGIC {
        return Err(Error::NotThisFormat);
    }
    let mut out = Vec::new();
    let mut at = MAGIC.len() as u64;
    while let Some(h) = usize::try_from(at).ok().and_then(|a| data.get(a..a.checked_add(HEADER)?)) {
        let m = header(at, h)?;
        at = m.next();
        out.push(m);
    }
    Ok(out)
}

/// The data of member `m` in the whole archive `data`.
pub fn data<'a>(data: &'a [u8], m: &Member) -> Result<&'a [u8], Error> {
    let start = usize::try_from(m.offset).map_err(|_| Error::Truncated)?;
    let len = usize::try_from(m.size).map_err(|_| Error::Truncated)?;
    data.get(start..start.checked_add(len).ok_or(Error::Truncated)?).ok_or(Error::Truncated)
}

/// An archive of `members` (name, data), as ar(1) writes it: for tests and
/// fixtures.
pub fn build(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    for (name, body) in members {
        let header = alloc::format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", 0, 0, 0, "100644", body.len());
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_are_listed_with_their_data() {
        let a = build(&[("debian-binary", b"2.0\n"), ("odd", b"abc"), ("last/", b"zz")]);
        let ms = members(&a).unwrap();
        let names: Vec<&str> = ms.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["debian-binary", "odd", "last"]);
        assert_eq!(data(&a, &ms[0]).unwrap(), b"2.0\n");
        assert_eq!(data(&a, &ms[1]).unwrap(), b"abc");
        // The odd member is padded: the next starts on an even offset.
        assert_eq!(ms[2].offset % 2, 0);
        assert_eq!(data(&a, &ms[2]).unwrap(), b"zz");
    }

    #[test]
    fn the_beginning_of_an_archive_lists_what_it_holds() {
        let a = build(&[("debian-binary", b"2.0\n"), ("control.tar", &[7u8; 300]), ("data.tar", &[9u8; 5000])]);
        // 8 + 60 + 4 + 60 = 132 bytes reach the second header.
        let ms = members(&a[..132]).unwrap();
        assert_eq!(ms.len(), 2);
        assert_eq!((ms[1].name.as_str(), ms[1].offset, ms[1].size), ("control.tar", 132, 300));
        // Its data is not there: asking for it says so.
        assert_eq!(data(&a[..132], &ms[1]), Err(Error::Truncated));
    }

    #[test]
    fn what_is_not_an_archive_is_refused() {
        assert_eq!(members(b"\x7fELF\x02\x01\x01\x00rest"), Err(Error::NotThisFormat));
        assert_eq!(members(b"!<ar"), Err(Error::Truncated));
        let mut a = build(&[("x", b"1234")]);
        a[8 + 58] = b'?';
        assert_eq!(members(&a), Err(Error::NotThisFormat));
        let mut a = build(&[("x", b"1234")]);
        a[8 + 48] = b'z';
        assert_eq!(members(&a), Err(Error::NotThisFormat));
    }
}
