//! Bounds-checked reading of little-endian and big-endian numbers, LEB128 and
//! C strings. Nothing here panics: every read says `None` or `Err` when the
//! bytes are not there.

use crate::error::{Error, Result};

/// Longest LEB128 number accepted: ceil(64 / 7) bytes.
pub(crate) const MAX_LEB_LEN: usize = 10;

/// `data[off..off + len]`, or `None` if it is not all inside `data`.
pub(crate) fn slice(data: &[u8], off: u64, len: u64) -> Option<&[u8]> {
    let end = off.checked_add(len)?;
    let off = usize::try_from(off).ok()?;
    let end = usize::try_from(end).ok()?;
    data.get(off..end)
}

pub(crate) fn u16_le(data: &[u8], off: usize) -> Option<u16> {
    let b = data.get(off..off.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

pub(crate) fn u32_le(data: &[u8], off: usize) -> Option<u32> {
    let b = data.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn u64_le(data: &[u8], off: usize) -> Option<u64> {
    let b = data.get(off..off.checked_add(8)?)?;
    Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

pub(crate) fn u32_be(data: &[u8], off: usize) -> Option<u32> {
    let b = data.get(off..off.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn u64_be(data: &[u8], off: usize) -> Option<u64> {
    let b = data.get(off..off.checked_add(8)?)?;
    Some(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// The bytes of the NUL-terminated string that starts at `data[off]`,
/// without the NUL. `None` if `off` is outside `data`, there is no NUL
/// before the end, or the string is longer than `max_len`.
pub(crate) fn cstr(data: &[u8], off: usize, max_len: usize) -> Option<&[u8]> {
    let rest = data.get(off..)?;
    let window = if rest.len() > max_len.saturating_add(1) { &rest[..max_len + 1] } else { rest };
    let n = window.iter().position(|&b| b == 0)?;
    Some(&window[..n])
}

/// A cursor over a byte stream with LEB128 reading.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    pub(crate) fn at(data: &'a [u8], pos: usize) -> Self {
        Cursor { data, pos }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub(crate) fn u8(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    /// A NUL-terminated string (the NUL is consumed, not returned).
    pub(crate) fn cstr(&mut self, max_len: usize) -> Option<&'a [u8]> {
        let s = cstr(self.data, self.pos, max_len)?;
        self.pos += s.len() + 1;
        Some(s)
    }

    /// An unsigned LEB128 number of at most 64 bits.
    pub(crate) fn uleb(&mut self) -> Result<u64> {
        let mut result = 0u64;
        let mut shift = 0u32;
        for _ in 0..MAX_LEB_LEN {
            let byte = self.u8().ok_or(Error::BadLeb128)?;
            let bits = u64::from(byte & 0x7f);
            if shift == 63 && bits > 1 {
                return Err(Error::BadLeb128);
            }
            result |= bits << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
        Err(Error::BadLeb128)
    }

    /// A signed LEB128 number of at most 64 bits.
    pub(crate) fn sleb(&mut self) -> Result<i64> {
        let mut result = 0i64;
        let mut shift = 0u32;
        for _ in 0..MAX_LEB_LEN {
            let byte = self.u8().ok_or(Error::BadLeb128)?;
            let bits = i64::from(byte & 0x7f);
            if shift == 63 {
                // Only the sign bit is left: the byte must be 0x00 (positive)
                // or 0x7f (negative), nothing else fits 64 bits.
                if byte != 0x00 && byte != 0x7f {
                    return Err(Error::BadLeb128);
                }
                if byte == 0x7f {
                    result |= -1i64 << 63;
                }
                return Ok(result);
            }
            result |= bits << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if byte & 0x40 != 0 {
                    result |= -1i64 << shift;
                }
                return Ok(result);
            }
        }
        Err(Error::BadLeb128)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uleb_values() {
        let mut c = Cursor::new(&[0x00, 0x7f, 0x80, 0x01, 0xe5, 0x8e, 0x26]);
        assert_eq!(c.uleb(), Ok(0));
        assert_eq!(c.uleb(), Ok(127));
        assert_eq!(c.uleb(), Ok(128));
        assert_eq!(c.uleb(), Ok(624_485));
        assert!(c.is_empty());
    }

    #[test]
    fn uleb_max_and_overflow() {
        let max = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        assert_eq!(Cursor::new(&max).uleb(), Ok(u64::MAX));
        let over = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
        assert_eq!(Cursor::new(&over).uleb(), Err(Error::BadLeb128));
        let long = [0x80; 11];
        assert_eq!(Cursor::new(&long).uleb(), Err(Error::BadLeb128));
        assert_eq!(Cursor::new(&[0x80]).uleb(), Err(Error::BadLeb128));
        assert_eq!(Cursor::new(&[]).uleb(), Err(Error::BadLeb128));
    }

    #[test]
    fn sleb_values() {
        let mut c = Cursor::new(&[0x02, 0x7e, 0xff, 0x00, 0x81, 0x7f, 0x80, 0x7f]);
        assert_eq!(c.sleb(), Ok(2));
        assert_eq!(c.sleb(), Ok(-2));
        assert_eq!(c.sleb(), Ok(127));
        assert_eq!(c.sleb(), Ok(-127));
        assert_eq!(c.sleb(), Ok(-128));
    }

    #[test]
    fn sleb_extremes() {
        let min = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7f];
        assert_eq!(Cursor::new(&min).sleb(), Ok(i64::MIN));
        let max = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
        assert_eq!(Cursor::new(&max).sleb(), Ok(i64::MAX));
        let bad = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        assert_eq!(Cursor::new(&bad).sleb(), Err(Error::BadLeb128));
        assert_eq!(Cursor::new(&[0x80; 12]).sleb(), Err(Error::BadLeb128));
    }

    #[test]
    fn cstr_bounds() {
        assert_eq!(cstr(b"ab\0cd\0", 3, 10), Some(&b"cd"[..]));
        assert_eq!(cstr(b"ab\0cd", 3, 10), None);
        assert_eq!(cstr(b"abc\0", 9, 10), None);
        assert_eq!(cstr(b"abc\0", 0, 2), None);
        assert_eq!(cstr(b"abc\0", 0, 3), Some(&b"abc"[..]));
    }

    #[test]
    fn slice_overflow() {
        assert_eq!(slice(&[1, 2, 3], u64::MAX, 2), None);
        assert_eq!(slice(&[1, 2, 3], 1, 2), Some(&[2, 3][..]));
        assert_eq!(slice(&[1, 2, 3], 1, 3), None);
    }
}
