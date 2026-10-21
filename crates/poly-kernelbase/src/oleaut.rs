//! The first functions of our `oleaut32.dll` and `user32.dll` (ADR 0087):
//! `BSTR`s and `VARIANT`s, and two string functions of `user32`, as far
//! as they can be decided without a process.
//!
//! Sources: Learn's pages of `SysAllocString`, `SysAllocStringLen`,
//! `SysFreeString`, `SysStringLen`, `VariantInit`, `VariantClear`,
//! `VariantCopy`, the "BSTR" and "VARIANT structure" pages, `CharUpperW`,
//! `CharPrevExA`; `wtypes.h` / `oaidl.h` for `VARTYPE` values and the
//! `VARIANT` layout, `winerror.h` for the `HRESULT`s. The ordinals
//! `oleaut32.dll` exports these at are export-table data (Wine's
//! `oleaut32.spec` at the coverage map's revision, ADR 0087).

/// A `BSTR` is a 4-byte length prefix (the string's length in bytes, the
/// terminator not counted), the characters, and a 2-byte NUL terminator;
/// the `BSTR` points at the first character (Learn, "BSTR").
pub const BSTR_PREFIX: usize = 4;

/// The bytes to allocate for a `BSTR` of `chars` characters (prefix,
/// characters, terminator), or `None` if its byte length does not fit the
/// 32-bit prefix.
pub fn bstr_alloc_size(chars: u32) -> Option<usize> {
    let bytes = u32::try_from(u64::from(chars) * 2).ok()?;
    Some(BSTR_PREFIX + bytes as usize + 2)
}

/// `sizeof(VARIANT)` on x64 (`oaidl.h`: `vt`, three reserved `WORD`s, and
/// a 16-byte union, the largest member being `BRECORD`'s two pointers).
pub const VARIANT_SIZE: usize = 24;
/// Where the value starts.
pub const VARIANT_VALUE: usize = 8;

/// `VARTYPE` values (`wtypes.h`).
pub const VT_EMPTY: u16 = 0;
pub const VT_BSTR: u16 = 8;
pub const VT_DISPATCH: u16 = 9;
pub const VT_VARIANT: u16 = 12;
pub const VT_UNKNOWN: u16 = 13;
pub const VT_DECIMAL: u16 = 14;
pub const VT_RECORD: u16 = 36;
pub const VT_VECTOR: u16 = 0x1000;
pub const VT_ARRAY: u16 = 0x2000;
pub const VT_BYREF: u16 = 0x4000;
pub const VT_RESERVED: u16 = 0x8000;
pub const VT_TYPEMASK: u16 = 0x0FFF;

/// `HRESULT`s (`winerror.h`).
pub const S_OK: u32 = 0;
pub const E_NOTIMPL: u32 = 0x8000_4001;
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
pub const DISP_E_BADVARTYPE: u32 = 0x8002_0008;

/// What a `VARIANT` of type `vt` holds, for `VariantClear` and
/// `VariantCopy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// A value copied by its bits (numbers, dates, booleans, `VT_EMPTY`,
    /// `VT_NULL`, `VT_DECIMAL`, and everything `VT_BYREF`: a reference the
    /// variant does not own, Learn).
    Bits,
    /// A `BSTR` it owns.
    Bstr,
    /// An interface pointer it holds a reference on (`IUnknown::AddRef`,
    /// vtable slot 1; `Release`, slot 2).
    Interface,
    /// An array or a record: Poly has no `SAFEARRAY` or `IRecordInfo` yet,
    /// so the call fails with `E_NOTIMPL` (Poly's choice; a stub, ADR 0087).
    Unsupported,
    /// Not a type a `VARIANT` may have: `DISP_E_BADVARTYPE`.
    Bad,
}

/// Whether `t` (without flags) is a type the "VARIANT structure" page lets
/// a `VARIANT` hold by value: `VT_EMPTY` to `VT_DECIMAL` but the
/// by-reference-only `VT_VARIANT`, `VT_I1` to `VT_UINT`, and `VT_RECORD`.
fn by_value(t: u16) -> bool {
    matches!(t, 0..=11 | 13 | 14 | 16..=23 | VT_RECORD)
}

/// Classify `vt`.
pub fn held(vt: u16) -> Held {
    let t = vt & VT_TYPEMASK;
    if vt & (VT_RESERVED | VT_VECTOR) != 0 {
        return Held::Bad;
    }
    if vt & VT_BYREF != 0 {
        return if by_value(t) || t == VT_VARIANT { Held::Bits } else { Held::Bad };
    }
    if vt & VT_ARRAY != 0 {
        return if by_value(t) || t == VT_VARIANT { Held::Unsupported } else { Held::Bad };
    }
    match t {
        VT_BSTR => Held::Bstr,
        VT_DISPATCH | VT_UNKNOWN => Held::Interface,
        VT_RECORD => Held::Unsupported,
        t if by_value(t) => Held::Bits,
        _ => Held::Bad,
    }
}

/// `CharUpperW`'s argument: Learn: "If the high-order word of this
/// parameter is zero, the low-order word must contain a single character".
pub fn is_single_char(arg: usize) -> bool {
    arg >> 16 == 0
}

/// `CharPrevExA` in a single-byte code page (every code page Poly has,
/// ADR 0079: 1252, 437 and UTF-8, which has no lead bytes in Learn's
/// sense): the byte before `current`, or `start` when `current` is at or
/// before it (Learn).
pub fn char_prev(start: usize, current: usize) -> usize {
    if current <= start { start } else { current - 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bstr_sizes() {
        assert_eq!(bstr_alloc_size(0), Some(6));
        assert_eq!(bstr_alloc_size(5), Some(4 + 10 + 2));
        assert_eq!(bstr_alloc_size(0x7FFF_FFFF), Some(4 + 0xFFFF_FFFE + 2));
        assert_eq!(bstr_alloc_size(0x8000_0000), None);
    }

    #[test]
    fn variant_types() {
        assert_eq!(held(VT_EMPTY), Held::Bits);
        assert_eq!(held(3), Held::Bits, "VT_I4");
        assert_eq!(held(VT_DECIMAL), Held::Bits);
        assert_eq!(held(VT_BSTR), Held::Bstr);
        assert_eq!(held(VT_UNKNOWN), Held::Interface);
        assert_eq!(held(VT_DISPATCH), Held::Interface);
        assert_eq!(held(VT_BSTR | VT_BYREF), Held::Bits);
        assert_eq!(held(VT_UNKNOWN | VT_BYREF), Held::Bits, "Learn: a reference is not released");
        assert_eq!(held(VT_VARIANT | VT_BYREF), Held::Bits);
        assert_eq!(held(VT_VARIANT), Held::Bad);
        assert_eq!(held(VT_BSTR | VT_ARRAY), Held::Unsupported);
        assert_eq!(held(VT_RECORD), Held::Unsupported);
        assert_eq!(held(15), Held::Bad);
        assert_eq!(held(64), Held::Bad, "VT_FILETIME is not a VARIANT type");
        assert_eq!(held(3 | VT_VECTOR), Held::Bad);
        assert_eq!(held(3 | VT_RESERVED), Held::Bad);
    }

    #[test]
    fn user32_strings() {
        assert!(is_single_char(0x61));
        assert!(is_single_char(0xFFFF));
        assert!(!is_single_char(0x1_0000));
        assert_eq!(char_prev(100, 105), 104);
        assert_eq!(char_prev(100, 100), 100);
        assert_eq!(char_prev(100, 90), 100);
    }
}
