//! The two checksums ext4 and its journal use, from their public
//! definitions.
//!
//! - **CRC-32C** (Castagnoli): the reflected polynomial `0x82F63B78`, as in
//!   RFC 3720 (iSCSI), appendix B.4. ext4's `metadata_csum` and jbd2's
//!   checksum v2/v3 chain it: each checksum starts from a seed and runs over
//!   several pieces, with no final inversion ([`crc32c`] returns the raw
//!   register). The standard CRC-32C of some bytes is `!crc32c(!0, bytes)`.
//! - **CRC-16** (the "ANSI"/IBM polynomial `0x8005`, reflected `0xA001`),
//!   which `uninit_bg` (`gdt_csum`) volumes use for group descriptors.

const fn table32() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const fn table16() -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u16;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xA001 } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC32C: [u32; 256] = table32();
static CRC16: [u16; 256] = table16();

/// CRC-32C over `data`, continuing from `seed` (no final inversion).
pub fn crc32c(seed: u32, data: &[u8]) -> u32 {
    let mut c = seed;
    for &b in data {
        c = CRC32C[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c
}

/// CRC-16 (reflected 0xA001) over `data`, continuing from `seed`.
pub fn crc16(seed: u16, data: &[u8]) -> u16 {
    let mut c = seed;
    for &b in data {
        c = CRC16[((c ^ b as u16) & 0xff) as usize] ^ (c >> 8);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_values() {
        // The published check values of the two algorithms ("123456789").
        assert_eq!(!crc32c(!0, b"123456789"), 0xE306_9283);
        assert_eq!(crc16(0, b"123456789"), 0xBB3D);
        // Chaining is the same as one run.
        assert_eq!(crc32c(crc32c(!0, b"1234"), b"56789"), crc32c(!0, b"123456789"));
        // RFC 3720 B.4: 32 bytes of zeros.
        assert_eq!(!crc32c(!0, &[0u8; 32]), 0x8A91_36AA);
    }
}
