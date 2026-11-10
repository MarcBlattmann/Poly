//! The sfnt container: the table directory of a TrueType/OpenType font or of
//! one font of a collection, and the file kinds `IDWriteFontFile::Analyze`
//! reports. Source: the OpenType specification ("OpenType font file").

use alloc::vec::Vec;

/// `DWRITE_FONT_FILE_TYPE` (`dwrite.h`).
pub mod file_type {
    pub const UNKNOWN: u32 = 0;
    pub const CFF: u32 = 1;
    pub const TRUETYPE: u32 = 2;
    pub const OPENTYPE_COLLECTION: u32 = 3;
}

/// `DWRITE_FONT_FACE_TYPE` (`dwrite.h`).
pub mod face_type {
    pub const CFF: u32 = 0;
    pub const TRUETYPE: u32 = 1;
    pub const OPENTYPE_COLLECTION: u32 = 2;
    pub const TYPE1: u32 = 3;
    pub const VECTOR: u32 = 4;
    pub const BITMAP: u32 = 5;
    pub const UNKNOWN: u32 = 6;
    pub const RAW_CFF: u32 = 7;
}

const TTC: u32 = 0x7474_6366; // 'ttcf'
const SFNT_TT: u32 = 0x0001_0000;
const SFNT_TRUE: u32 = 0x7472_7565; // 'true'
const SFNT_OTTO: u32 = 0x4F54_544F; // 'OTTO'

pub fn be16(d: &[u8], at: usize) -> Option<u16> {
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

pub fn be16s(d: &[u8], at: usize) -> Option<i16> {
    be16(d, at).map(|v| v as i16)
}

pub fn be32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// What `Analyze` finds out about a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Analysis {
    pub supported: bool,
    pub file_type: u32,
    pub face_type: u32,
    pub faces: u32,
}

impl Analysis {
    pub const UNSUPPORTED: Analysis =
        Analysis { supported: false, file_type: file_type::UNKNOWN, face_type: face_type::UNKNOWN, faces: 0 };
}

/// Check the table directory at `offset`: the version tag, at least one
/// table and the whole directory inside the file.
fn directory_ok(data: &[u8], offset: usize) -> Option<(u32, usize)> {
    let version = be32(data, offset)?;
    if !matches!(version, SFNT_TT | SFNT_TRUE | SFNT_OTTO) {
        return None;
    }
    let n = usize::from(be16(data, offset + 4)?);
    if n == 0 {
        return None;
    }
    let end = offset.checked_add(12 + 16 * n)?;
    if end > data.len() {
        return None;
    }
    Some((version, n))
}

/// What kind of font file `data` is.
pub fn analyze(data: &[u8]) -> Analysis {
    let Some(tag) = be32(data, 0) else { return Analysis::UNSUPPORTED };
    if tag == TTC {
        let Some(count) = be32(data, 8) else { return Analysis::UNSUPPORTED };
        if count == 0 || count > 0x10000 {
            return Analysis::UNSUPPORTED;
        }
        let Some(first) = be32(data, 12) else { return Analysis::UNSUPPORTED };
        if directory_ok(data, first as usize).is_none() {
            return Analysis::UNSUPPORTED;
        }
        return Analysis {
            supported: true,
            file_type: file_type::OPENTYPE_COLLECTION,
            face_type: face_type::OPENTYPE_COLLECTION,
            faces: count,
        };
    }
    match directory_ok(data, 0) {
        Some((SFNT_OTTO, _)) => {
            Analysis { supported: true, file_type: file_type::CFF, face_type: face_type::CFF, faces: 1 }
        }
        Some(_) => {
            Analysis { supported: true, file_type: file_type::TRUETYPE, face_type: face_type::TRUETYPE, faces: 1 }
        }
        None => Analysis::UNSUPPORTED,
    }
}

/// One font of a file: its tables as slices.
#[derive(Clone, Debug)]
pub struct Sfnt<'a> {
    data: &'a [u8],
    tables: Vec<([u8; 4], usize, usize)>,
    pub cff: bool,
}

impl<'a> Sfnt<'a> {
    /// Font `index` of `data` (0 for a plain font file).
    pub fn parse(data: &'a [u8], index: u32) -> Option<Sfnt<'a>> {
        let tag = be32(data, 0)?;
        let offset = if tag == TTC {
            let count = be32(data, 8)?;
            if index >= count {
                return None;
            }
            be32(data, 12 + 4 * index as usize)? as usize
        } else {
            if index != 0 {
                return None;
            }
            0
        };
        let (version, n) = directory_ok(data, offset)?;
        let mut tables = Vec::with_capacity(n);
        for i in 0..n {
            let e = offset + 12 + 16 * i;
            let t = data.get(e..e + 4)?;
            let off = be32(data, e + 8)? as usize;
            let len = be32(data, e + 12)? as usize;
            if off.checked_add(len).is_none_or(|end| end > data.len()) {
                continue;
            }
            tables.push(([t[0], t[1], t[2], t[3]], off, len));
        }
        Some(Sfnt { data, tables, cff: version == SFNT_OTTO })
    }

    /// The bytes of table `tag`.
    pub fn table(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        self.tables.iter().find(|t| &t.0 == tag).map(|&(_, off, len)| &self.data[off..off + len])
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn rejects_non_fonts() {
        assert_eq!(analyze(b"hello world, this is not a font at all"), Analysis::UNSUPPORTED);
        assert_eq!(analyze(&[0, 1, 0, 0]), Analysis::UNSUPPORTED);
        let mut otto = vec![0u8; 12];
        otto[..4].copy_from_slice(b"OTTO");
        assert_eq!(analyze(&otto), Analysis::UNSUPPORTED);
        let mut ttc = vec![0u8; 12];
        ttc[..4].copy_from_slice(b"ttcf");
        assert_eq!(analyze(&ttc), Analysis::UNSUPPORTED);
    }

    #[test]
    fn accepts_a_directory() {
        let mut f = vec![0u8; 12 + 16];
        f[..4].copy_from_slice(&[0, 1, 0, 0]);
        f[4..6].copy_from_slice(&1u16.to_be_bytes());
        let a = analyze(&f);
        assert!(a.supported);
        assert_eq!((a.file_type, a.face_type, a.faces), (file_type::TRUETYPE, face_type::TRUETYPE, 1));
        let cut = &f[..20];
        assert!(!analyze(cut).supported);
    }
}
