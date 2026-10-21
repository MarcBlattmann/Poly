//! Code units: `char` (bytes) and `wchar_t` (UTF-16 units on Windows), so
//! the narrow and wide functions (`strtol` / `wcstol`, `sprintf` /
//! `swprintf`) share one implementation.

/// A code unit of a C string.
pub trait Unit: Copy + Eq + Ord + Default + core::fmt::Debug + 'static {
    /// Whether this is `wchar_t`.
    const WIDE: bool;
    /// The unit with this value (truncated).
    fn from_u32(c: u32) -> Self;
    /// The unit's value, zero-extended.
    fn to_u32(self) -> u32;
    /// An ASCII character as a unit.
    fn ascii(b: u8) -> Self {
        Self::from_u32(u32::from(b))
    }
    /// Whether this is NUL.
    fn is_nul(self) -> bool {
        self.to_u32() == 0
    }
    /// The unit as an ASCII byte, if it is one.
    fn as_ascii(self) -> Option<u8> {
        let v = self.to_u32();
        if v < 0x80 { Some(v as u8) } else { None }
    }
}

impl Unit for u8 {
    const WIDE: bool = false;
    fn from_u32(c: u32) -> Self {
        c as u8
    }
    fn to_u32(self) -> u32 {
        u32::from(self)
    }
}

impl Unit for u16 {
    const WIDE: bool = true;
    fn from_u32(c: u32) -> Self {
        c as u16
    }
    fn to_u32(self) -> u32 {
        u32::from(self)
    }
}

/// The length of a NUL-terminated string, in units.
///
/// # Safety
/// `s` points to a NUL-terminated string.
pub unsafe fn len<C: Unit>(s: *const C) -> usize {
    let mut n = 0;
    // SAFETY: the caller's string is readable up to its NUL.
    while !unsafe { *s.add(n) }.is_nul() {
        n += 1;
    }
    n
}

/// A NUL-terminated string as a slice, without the NUL.
///
/// # Safety
/// `s` points to a NUL-terminated string that outlives `'a`.
pub unsafe fn slice<'a, C: Unit>(s: *const C) -> &'a [C] {
    // SAFETY: the caller's contract; `len` units are readable.
    unsafe { core::slice::from_raw_parts(s, len(s)) }
}

/// At most `max` units of a string that may lack its NUL (`strnlen`).
///
/// # Safety
/// `s` is readable up to its NUL or `max` units, whichever comes first.
pub unsafe fn slice_max<'a, C: Unit>(s: *const C, max: usize) -> &'a [C] {
    let mut n = 0;
    // SAFETY: the caller's contract.
    while n < max && !unsafe { *s.add(n) }.is_nul() {
        n += 1;
    }
    // SAFETY: `n` units were just read.
    unsafe { core::slice::from_raw_parts(s, n) }
}

/// The "C" locale's narrow ↔ wide conversions (`mbtowc` / `wctomb`): a
/// byte is the code point of the same value, and a wide character above
/// 0xFF has no narrow form (to be confirmed by the spec agent: Poly reads
/// the "C" locale's `LC_CTYPE` as ISO-8859-1, so `%ls` of other characters
/// fails with `EILSEQ`).
pub fn c_locale_to_wide(b: u8) -> u16 {
    u16::from(b)
}

/// See [`c_locale_to_wide`].
pub fn c_locale_to_narrow(c: u16) -> Option<u8> {
    u8::try_from(c).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        let s = b"abc\0def";
        // SAFETY: NUL-terminated.
        assert_eq!(unsafe { len(s.as_ptr()) }, 3);
        let w: [u16; 3] = [0x41, 0x100, 0];
        // SAFETY: NUL-terminated.
        assert_eq!(unsafe { slice(w.as_ptr()) }, &[0x41, 0x100]);
        // SAFETY: four readable bytes.
        assert_eq!(unsafe { slice_max(b"abcd".as_ptr(), 2) }, b"ab");
        assert_eq!(c_locale_to_narrow(0xE9), Some(0xE9));
        assert_eq!(c_locale_to_narrow(0x20AC), None);
        const { assert!(<u16 as Unit>::WIDE && !<u8 as Unit>::WIDE) };
        assert_eq!(u8::ascii(b'x').as_ascii(), Some(b'x'));
        assert_eq!(0xE9u8.as_ascii(), None);
    }
}
