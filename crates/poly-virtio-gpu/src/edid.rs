//! The parts of an EDID base block (VESA E-EDID, version 1.3 / 1.4) a
//! display driver wants: who made the monitor, its name, and its
//! preferred timing.
//!
//! A base block is 128 bytes: the fixed header `00 FF FF FF FF FF FF 00`,
//! the vendor (bytes 8-9: three letters of five bits each, 'A' = 1,
//! big-endian), product code (10-11, little-endian), version and revision
//! (18-19), and four 18-byte descriptors from byte 54. The first
//! descriptor is the preferred timing when its pixel clock (two bytes,
//! little-endian, in 10 kHz) is not zero; its horizontal active pixels are
//! byte 2 plus the high nibble of byte 4 as bits 8-11, its vertical active
//! lines byte 5 plus the high nibble of byte 7. A descriptor with a zero
//! pixel clock is a display descriptor: tag 0xFC is the monitor name, up
//! to 13 characters ended by a line feed. All 128 bytes sum to 0 modulo
//! 256. Extension blocks (byte 126 counts them) are not read.

/// Bytes of a base block.
pub const BLOCK_LEN: usize = 128;
const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
const DESCRIPTORS: usize = 54;
const DESCRIPTOR_LEN: usize = 18;
const TAG_NAME: u8 = 0xfc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdidError {
    /// Fewer than 128 bytes.
    TooShort(usize),
    /// The fixed header is wrong.
    BadHeader,
    /// The block's bytes do not sum to 0.
    BadChecksum,
}

/// What a base block says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edid {
    /// The three-letter vendor ID (PNP ID), e.g. `*b"RHT"`.
    pub vendor: [u8; 3],
    pub product: u16,
    /// EDID version and revision, e.g. (1, 4).
    pub version: (u8, u8),
    /// Active pixels and lines of the preferred timing, if the first
    /// descriptor is one.
    pub preferred: Option<(u32, u32)>,
    /// The monitor name descriptor, as stored (ASCII), and its length.
    name: [u8; 13],
    name_len: usize,
}

impl Edid {
    /// Parse the base block at the start of `b`.
    pub fn parse(b: &[u8]) -> Result<Edid, EdidError> {
        let block = b.get(..BLOCK_LEN).ok_or(EdidError::TooShort(b.len()))?;
        if block[..8] != HEADER {
            return Err(EdidError::BadHeader);
        }
        if block.iter().fold(0u8, |s, &x| s.wrapping_add(x)) != 0 {
            return Err(EdidError::BadChecksum);
        }
        let id = u16::from_be_bytes([block[8], block[9]]);
        let letter = |shift: u16| {
            let v = ((id >> shift) & 0x1f) as u8;
            if (1..=26).contains(&v) { b'A' + v - 1 } else { b'?' }
        };
        let mut e = Edid {
            vendor: [letter(10), letter(5), letter(0)],
            product: u16::from_le_bytes([block[10], block[11]]),
            version: (block[18], block[19]),
            preferred: None,
            name: [0; 13],
            name_len: 0,
        };
        for i in 0..4 {
            let d = &block[DESCRIPTORS + i * DESCRIPTOR_LEN..][..DESCRIPTOR_LEN];
            let clock = u16::from_le_bytes([d[0], d[1]]);
            if clock != 0 {
                if i == 0 {
                    let w = d[2] as u32 | ((d[4] as u32 & 0xf0) << 4);
                    let h = d[5] as u32 | ((d[7] as u32 & 0xf0) << 4);
                    e.preferred = (w > 0 && h > 0).then_some((w, h));
                }
            } else if d[3] == TAG_NAME {
                let text = &d[5..];
                let len = text.iter().position(|&c| c == b'\n').unwrap_or(text.len());
                e.name[..len].copy_from_slice(&text[..len]);
                e.name_len = len;
            }
        }
        Ok(e)
    }

    /// The monitor name, if the block has one in printable ASCII.
    pub fn name(&self) -> Option<&str> {
        let n = &self.name[..self.name_len];
        if n.is_empty() || !n.iter().all(|c| (0x20..0x7f).contains(c)) {
            return None;
        }
        core::str::from_utf8(n).ok().map(str::trim_end)
    }

    pub fn vendor(&self) -> &str {
        core::str::from_utf8(&self.vendor).unwrap_or("???")
    }
}
