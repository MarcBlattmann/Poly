//! `GUID`s as text (Learn: "StringFromGUID2", "CLSIDFromString",
//! "IIDFromString"; `guiddef.h` for the layout).

/// A `GUID` (`guiddef.h`): 16 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

/// The text form, `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`, in characters
/// (without the terminating NUL); `StringFromGUID2` needs one more.
pub const GUID_TEXT_LEN: usize = 38;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

impl Guid {
    /// The braced text in upper case, as `StringFromGUID2` writes it.
    pub fn to_text(&self) -> [u16; GUID_TEXT_LEN] {
        let mut out = [0u16; GUID_TEXT_LEN];
        let mut n = 0;
        let mut put = |c: u8| {
            out[n] = u16::from(c);
            n += 1;
        };
        put(b'{');
        for s in (0..8).rev() {
            put(HEX[((self.data1 >> (s * 4)) & 15) as usize]);
        }
        put(b'-');
        for s in (0..4).rev() {
            put(HEX[((self.data2 >> (s * 4)) & 15) as usize]);
        }
        put(b'-');
        for s in (0..4).rev() {
            put(HEX[((self.data3 >> (s * 4)) & 15) as usize]);
        }
        put(b'-');
        for (i, b) in self.data4.iter().enumerate() {
            if i == 2 {
                put(b'-');
            }
            put(HEX[(b >> 4) as usize]);
            put(HEX[(b & 15) as usize]);
        }
        put(b'}');
        out
    }

    /// Parse the braced text; hex digits of either case. `None` for any
    /// other form (no braces, wrong length, a character that is not hex).
    pub fn parse(text: &[u16]) -> Option<Guid> {
        if text.len() != GUID_TEXT_LEN || text[0] != u16::from(b'{') || text[GUID_TEXT_LEN - 1] != u16::from(b'}') {
            return None;
        }
        for &i in &[9usize, 14, 19, 24] {
            if text[i] != u16::from(b'-') {
                return None;
            }
        }
        let hex = |from: usize, digits: usize| -> Option<u64> {
            let mut v = 0u64;
            for &c in &text[from..from + digits] {
                let d = match u8::try_from(c).ok()? {
                    c @ b'0'..=b'9' => c - b'0',
                    c @ b'a'..=b'f' => c - b'a' + 10,
                    c @ b'A'..=b'F' => c - b'A' + 10,
                    _ => return None,
                };
                v = v << 4 | u64::from(d);
            }
            Some(v)
        };
        let data1 = hex(1, 8)? as u32;
        let data2 = hex(10, 4)? as u16;
        let data3 = hex(15, 4)? as u16;
        let mut data4 = [0u8; 8];
        data4[0] = hex(20, 2)? as u8;
        data4[1] = hex(22, 2)? as u8;
        for (i, b) in data4[2..].iter_mut().enumerate() {
            *b = hex(25 + i * 2, 2)? as u8;
        }
        Some(Guid { data1, data2, data3, data4 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn w(s: &str) -> alloc::vec::Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn text_round_trip_in_upper_case() {
        let g = Guid::parse(&w("{00000000-0000-0000-C000-000000000046}")).unwrap();
        assert_eq!(g.data1, 0);
        assert_eq!(g.data4, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
        let t = g.to_text();
        assert_eq!(String::from_utf16(&t).unwrap(), "{00000000-0000-0000-C000-000000000046}");
        let l = Guid::parse(&w("{0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9}")).unwrap();
        assert_eq!(String::from_utf16(&l.to_text()).unwrap(), "{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}");
        assert_eq!(l.data1, 0x0a1b_2c3d);
        assert_eq!(l.data3, 0x6071);
    }

    #[test]
    fn sheet_forms_parse_to_the_documented_bytes() {
        let g = Guid::parse(&w("{0000000a-bcde-f012-C000-00000000004f}")).unwrap();
        let b = [
            g.data1.to_le_bytes().as_slice(),
            g.data2.to_le_bytes().as_slice(),
            g.data3.to_le_bytes().as_slice(),
            g.data4.as_slice(),
        ]
        .concat();
        assert_eq!(b, [0x0a, 0, 0, 0, 0xde, 0xbc, 0x12, 0xf0, 0xc0, 0, 0, 0, 0, 0, 0, 0x4f]);
        let m = Guid::parse(&w("{12345678-9ABC-DEF0-1122-334455667788}")).unwrap();
        assert_eq!(m.data1, 0x1234_5678);
        assert_eq!(m.data2, 0x9abc);
        assert_eq!(m.data3, 0xdef0);
        assert_eq!(m.data4, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        assert_eq!(String::from_utf16(&m.to_text()).unwrap(), "{12345678-9ABC-DEF0-1122-334455667788}");
        let ones = Guid { data1: !0, data2: !0, data3: !0, data4: [0xFF; 8] };
        assert_eq!(String::from_utf16(&ones.to_text()).unwrap(), "{FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF}");
        assert_eq!(String::from_utf16(&Guid::default().to_text()).unwrap(), "{00000000-0000-0000-0000-000000000000}");
    }

    #[test]
    fn white_space_around_and_missing_dashes_are_refused() {
        for bad in [
            " {00000000-0000-0000-C000-000000000046}",
            "{00000000-0000-0000-C000-000000000046} ",
            "{00000000000000000000000000000000000000}",
            "{00000000-0000-0000-C000-0000000000466}",
        ] {
            assert_eq!(Guid::parse(&w(bad)), None, "{bad}");
        }
    }

    #[test]
    fn rejects_other_forms() {
        for bad in [
            "",
            "00000000-0000-0000-C000-000000000046",
            "{00000000-0000-0000-C000-00000000004}",
            "{00000000_0000-0000-C000-000000000046}",
            "{0000000G-0000-0000-C000-000000000046}",
            "{00000000-0000-0000-C000-000000000046",
        ] {
            assert_eq!(Guid::parse(&w(bad)), None, "{bad}");
        }
    }
}
