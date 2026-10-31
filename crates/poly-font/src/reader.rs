//! Reading big-endian values out of font data without ever panicking:
//! every read past the end is `None`.

/// A cursor over a byte slice.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }

    /// A reader starting `offset` bytes into `data` (`None` past its end).
    pub fn at(data: &'a [u8], offset: usize) -> Option<Reader<'a>> {
        (offset <= data.len()).then_some(Reader { data, pos: offset })
    }

    pub fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.pos.checked_add(n)?;
        if end > self.data.len() {
            return None;
        }
        self.pos = end;
        Some(())
    }

    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let b = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(b)
    }

    pub fn u8(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    pub fn i8(&mut self) -> Option<i8> {
        self.u8().map(|b| b as i8)
    }

    pub fn u16(&mut self) -> Option<u16> {
        let b = self.bytes(2)?;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }

    pub fn u32(&mut self) -> Option<u32> {
        let b = self.bytes(4)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }

    /// A 2.14 fixed-point number (`F2DOT14`).
    pub fn f2dot14(&mut self) -> Option<f32> {
        self.i16().map(|v| f32::from(v) / 16384.0)
    }

    /// A 16.16 fixed-point number (`Fixed`).
    pub fn fixed(&mut self) -> Option<f32> {
        self.i32().map(|v| v as f32 / 65536.0)
    }
}

/// The `u16` at `offset` in `data`.
pub(crate) fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Reader::at(data, offset)?.u16()
}

/// The `i16` at `offset` in `data`.
pub(crate) fn i16_at(data: &[u8], offset: usize) -> Option<i16> {
    Reader::at(data, offset)?.i16()
}

/// The `u32` at `offset` in `data`.
pub(crate) fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Reader::at(data, offset)?.u32()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_big_endian_and_stops_at_the_end() {
        let data = [0x12, 0x34, 0xff, 0xfe, 0x00, 0x01, 0x40, 0x00];
        let mut r = Reader::new(&data);
        assert_eq!(r.u16(), Some(0x1234));
        assert_eq!(r.i16(), Some(-2));
        assert_eq!(r.u16(), Some(1));
        assert_eq!(r.f2dot14(), Some(1.0));
        assert_eq!(r.u8(), None);
        assert_eq!(u32_at(&data, 0), Some(0x1234_fffe));
        assert_eq!(u32_at(&data, 6), None);
        assert_eq!(u16_at(&data, usize::MAX), None);
        assert!(Reader::at(&data, 9).is_none());
        let mut r = Reader::new(&data);
        assert_eq!(r.skip(usize::MAX), None);
        assert_eq!(r.skip(7), Some(()));
        assert_eq!(r.i8(), Some(0));
    }
}
