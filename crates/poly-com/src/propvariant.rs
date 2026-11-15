//! `PROPVARIANT` types (Learn: "PROPVARIANT", "PropVariantClear",
//! "PropVariantCopy"; `wtypes.h` for the `VT_*` values, `propidl.h` for the
//! layout): which types exist, what a value of each owns, and the size of a
//! vector's elements. The DLL does the memory work with this table.
//!
//! Layout on x64 (24 bytes): `vt` at 0, three reserved words, the value at 8
//! (a pointer, a number, or a counted array: `cElems` as a 32-bit count at 8
//! and `pElems` at 16; a blob: `cbSize` at 8 and `pBlobData` at 16).

pub const VT_EMPTY: u16 = 0;
pub const VT_NULL: u16 = 1;
pub const VT_I2: u16 = 2;
pub const VT_I4: u16 = 3;
pub const VT_R4: u16 = 4;
pub const VT_R8: u16 = 5;
pub const VT_CY: u16 = 6;
pub const VT_DATE: u16 = 7;
pub const VT_BSTR: u16 = 8;
pub const VT_DISPATCH: u16 = 9;
pub const VT_ERROR: u16 = 10;
pub const VT_BOOL: u16 = 11;
pub const VT_VARIANT: u16 = 12;
pub const VT_UNKNOWN: u16 = 13;
pub const VT_DECIMAL: u16 = 14;
pub const VT_I1: u16 = 16;
pub const VT_UI1: u16 = 17;
pub const VT_UI2: u16 = 18;
pub const VT_UI4: u16 = 19;
pub const VT_I8: u16 = 20;
pub const VT_UI8: u16 = 21;
pub const VT_INT: u16 = 22;
pub const VT_UINT: u16 = 23;
pub const VT_LPSTR: u16 = 30;
pub const VT_LPWSTR: u16 = 31;
pub const VT_FILETIME: u16 = 64;
pub const VT_BLOB: u16 = 65;
pub const VT_STREAM: u16 = 66;
pub const VT_STORAGE: u16 = 67;
pub const VT_STREAMED_OBJECT: u16 = 68;
pub const VT_STORED_OBJECT: u16 = 69;
pub const VT_BLOB_OBJECT: u16 = 70;
pub const VT_CF: u16 = 71;
pub const VT_CLSID: u16 = 72;
pub const VT_VERSIONED_STREAM: u16 = 73;
pub const VT_VECTOR: u16 = 0x1000;
pub const VT_ARRAY: u16 = 0x2000;
pub const VT_BYREF: u16 = 0x4000;

/// What a value of one type owns, and so what clearing or copying it must do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The 8 value bytes (or 16, for a few) are the value: nothing to free.
    Plain,
    /// A `BSTR` (`SysFreeString`).
    Bstr,
    /// A `CoTaskMemAlloc` string, `LPSTR` or `LPWSTR`.
    TaskString,
    /// A `CoTaskMemAlloc` block of 16 bytes (`pclsid`).
    Clsid,
    /// `cbSize` bytes in a `CoTaskMemAlloc` block (`blob`).
    Blob,
    /// A COM interface pointer (`Release`).
    Interface,
    /// A `CLIPDATA` pointer (`pclipdata`: a count, a format and a block).
    ClipData,
    /// A counted array of this element kind.
    Vector(Elem),
    /// A `SAFEARRAY` (`SafeArrayDestroy`).
    SafeArray,
    /// A pointer to a value somebody else owns.
    ByRef,
}

/// The element of a counted vector (`CAxxx`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    /// `bytes` per element, nothing owned.
    Plain(usize),
    Bstr,
    TaskString,
    /// `PROPVARIANT` elements, each cleared in turn.
    Variant,
    /// `CLIPDATA` structures (16 bytes: size, format, data pointer).
    ClipData,
}

impl Elem {
    /// The element's size in bytes.
    pub fn size(self) -> usize {
        match self {
            Elem::Plain(n) => n,
            Elem::Bstr | Elem::TaskString => 8,
            Elem::Variant => 24,
            Elem::ClipData => 16,
        }
    }
}

/// The element a vector of `base` holds, `None` when `base` has no vector form.
pub fn vector_elem(base: u16) -> Option<Elem> {
    Some(match base {
        VT_I1 | VT_UI1 => Elem::Plain(1),
        VT_I2 | VT_UI2 | VT_BOOL => Elem::Plain(2),
        VT_I4 | VT_UI4 | VT_R4 | VT_ERROR => Elem::Plain(4),
        VT_I8 | VT_UI8 | VT_R8 | VT_CY | VT_DATE | VT_FILETIME => Elem::Plain(8),
        VT_CLSID => Elem::Plain(16),
        VT_BSTR => Elem::Bstr,
        VT_LPSTR | VT_LPWSTR => Elem::TaskString,
        VT_VARIANT => Elem::Variant,
        VT_CF => Elem::ClipData,
        _ => return None,
    })
}

/// Whether `base` can be the element type of a `SAFEARRAY` in a variant.
fn array_base(base: u16) -> bool {
    matches!(
        base,
        VT_I2
            | VT_I4
            | VT_R4
            | VT_R8
            | VT_CY
            | VT_DATE
            | VT_BSTR
            | VT_DISPATCH
            | VT_ERROR
            | VT_BOOL
            | VT_VARIANT
            | VT_UNKNOWN
            | VT_DECIMAL
            | VT_I1
            | VT_UI1
            | VT_UI2
            | VT_UI4
            | VT_INT
            | VT_UINT
    )
}

/// The kind of an unmodified (no vector, array or by-reference flag) type; `None` for a type no
/// `PROPVARIANT` has.
fn base_kind(vt: u16) -> Option<Kind> {
    Some(match vt {
        VT_EMPTY | VT_NULL | VT_I2 | VT_I4 | VT_R4 | VT_R8 | VT_CY | VT_DATE | VT_ERROR | VT_BOOL | VT_DECIMAL
        | VT_I1 | VT_UI1 | VT_UI2 | VT_UI4 | VT_I8 | VT_UI8 | VT_INT | VT_UINT | VT_FILETIME => Kind::Plain,
        VT_BSTR => Kind::Bstr,
        VT_LPSTR | VT_LPWSTR => Kind::TaskString,
        VT_CLSID => Kind::Clsid,
        VT_BLOB | VT_BLOB_OBJECT => Kind::Blob,
        VT_UNKNOWN | VT_DISPATCH | VT_STREAM | VT_STORAGE | VT_STREAMED_OBJECT | VT_STORED_OBJECT => Kind::Interface,
        VT_CF => Kind::ClipData,
        // A versioned stream is a pointer to a structure holding a GUID and an `IStream`; it owns both.
        VT_VERSIONED_STREAM => Kind::Plain,
        _ => return None,
    })
}

/// What a variant of type `vt` owns; `None` when no `PROPVARIANT` has that type.
pub fn classify(vt: u16) -> Option<Kind> {
    let flags = vt & (VT_VECTOR | VT_ARRAY | VT_BYREF);
    let base = vt & !(VT_VECTOR | VT_ARRAY | VT_BYREF);
    match flags {
        0 => base_kind(base),
        VT_VECTOR => vector_elem(base).map(Kind::Vector),
        VT_ARRAY => array_base(base).then_some(Kind::SafeArray),
        VT_BYREF => (base_kind(base).is_some() || base == VT_VARIANT || array_base(base)).then_some(Kind::ByRef),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_own_nothing() {
        for vt in [VT_EMPTY, VT_I4, VT_BOOL, VT_R8, VT_FILETIME, VT_UI8, VT_NULL] {
            assert_eq!(classify(vt), Some(Kind::Plain), "{vt}");
        }
    }

    #[test]
    fn strings_blobs_and_interfaces_are_owned() {
        assert_eq!(classify(VT_BSTR), Some(Kind::Bstr));
        assert_eq!(classify(VT_LPWSTR), Some(Kind::TaskString));
        assert_eq!(classify(VT_LPSTR), Some(Kind::TaskString));
        assert_eq!(classify(VT_CLSID), Some(Kind::Clsid));
        assert_eq!(classify(VT_BLOB), Some(Kind::Blob));
        assert_eq!(classify(VT_UNKNOWN), Some(Kind::Interface));
        assert_eq!(classify(VT_STREAM), Some(Kind::Interface));
        assert_eq!(classify(VT_CF), Some(Kind::ClipData));
    }

    #[test]
    fn vectors_know_their_element() {
        assert_eq!(classify(VT_VECTOR | VT_I4), Some(Kind::Vector(Elem::Plain(4))));
        assert_eq!(classify(VT_VECTOR | VT_UI1), Some(Kind::Vector(Elem::Plain(1))));
        assert_eq!(classify(VT_VECTOR | VT_BSTR), Some(Kind::Vector(Elem::Bstr)));
        assert_eq!(classify(VT_VECTOR | VT_VARIANT), Some(Kind::Vector(Elem::Variant)));
        assert_eq!(Elem::Variant.size(), 24);
        assert_eq!(Elem::Plain(16).size(), 16);
        // no vector of interfaces, blobs or empties
        assert_eq!(classify(VT_VECTOR | VT_UNKNOWN), None);
        assert_eq!(classify(VT_VECTOR | VT_BLOB), None);
        assert_eq!(classify(VT_VECTOR | VT_EMPTY), None);
    }

    #[test]
    fn arrays_and_references() {
        assert_eq!(classify(VT_ARRAY | VT_I4), Some(Kind::SafeArray));
        assert_eq!(classify(VT_ARRAY | VT_BSTR), Some(Kind::SafeArray));
        assert_eq!(classify(VT_ARRAY | VT_LPWSTR), None);
        assert_eq!(classify(VT_BYREF | VT_I4), Some(Kind::ByRef));
        assert_eq!(classify(VT_VECTOR | VT_ARRAY | VT_I4), None);
    }

    #[test]
    fn types_nothing_defines_are_refused() {
        for vt in [0x7FFF, 15, 24, 29, 32, 63, 74, 0xFFF, 0x8000, 0x0100] {
            assert_eq!(classify(vt), None, "{vt:#x}");
        }
    }
}
