//! Safe arrays (ADR 0155, step 9): the part of `oleaut32.dll` Chrome's `chrome.dll` delay-loads
//! by ordinal (`SafeArrayCreateVector` 411, `SafeArrayDestroy` 16, `SafeArrayGetDim` 17,
//! `SafeArrayGetUBound` 19, `SafeArrayGetLBound` 20, `SafeArrayAccessData` 23,
//! `SafeArrayUnaccessData` 24, `SafeArrayGetElement` 25, `SafeArrayPutElement` 26,
//! `SafeArrayGetVartype` 77). A delay load of an ordinal the DLL does not export ends the browser
//! (`ERROR_INVALID_ORDINAL`).
//!
//! Sources: Learn, "SAFEARRAY structure (oaidl.h)" and "SAFEARRAYBOUND" (the descriptor: `cDims`,
//! `fFeatures`, `cbElements`, `cLocks`, `pvData`, one bound per dimension), the `FADF_*` flags of
//! `oaidl.h`, and the pages of the ten functions; `wtypes.h` for `VARTYPE`. The ordinals are
//! export-table data. Where the array's variant type is kept is Poly's own choice (a block in
//! front of the descriptor that only `oleaut32` reads). Doc-derived, not observed on Windows.

use crate::oleaut::{E_INVALIDARG, VT_BSTR, VT_DISPATCH, VT_RECORD, VT_UNKNOWN, VT_VARIANT};

/// `DISP_E_BADINDEX`, `DISP_E_ARRAYISLOCKED`, `E_UNEXPECTED` (`winerror.h`).
pub const DISP_E_BADINDEX: u32 = 0x8002_000B;
pub const DISP_E_ARRAYISLOCKED: u32 = 0x8002_000D;
pub const E_UNEXPECTED: u32 = 0x8000_FFFF;
/// `TYPE_E_LIBNOTREGISTERED` (`winerror.h`): `LoadRegTypeLib` on a system with no type library
/// registered.
pub const TYPE_E_LIBNOTREGISTERED: u32 = 0x8002_801D;

/// `FADF_*` (`oaidl.h`).
pub const FADF_RECORD: u16 = 0x0020;
pub const FADF_HAVEVARTYPE: u16 = 0x0080;
pub const FADF_BSTR: u16 = 0x0100;
pub const FADF_UNKNOWN: u16 = 0x0200;
pub const FADF_DISPATCH: u16 = 0x0400;
pub const FADF_VARIANT: u16 = 0x0800;

/// Offsets in a `SAFEARRAY` on x64 (`oaidl.h`): `cDims` (u16), `fFeatures` (u16), `cbElements`
/// (u32), `cLocks` (u32), `pvData` (pointer, aligned to 8), `rgsabound` (`cElements` u32,
/// `lLbound` i32 per dimension).
pub const SA_DIMS: usize = 0;
pub const SA_FEATURES: usize = 2;
pub const SA_ELEMENT_SIZE: usize = 4;
pub const SA_LOCKS: usize = 8;
pub const SA_DATA: usize = 16;
pub const SA_BOUNDS: usize = 24;
/// The size of a descriptor with one dimension.
pub const SA_VECTOR_SIZE: usize = SA_BOUNDS + 8;
/// The bytes Poly keeps in front of a descriptor it made: the array's `VARTYPE` in the first
/// four (16 bytes, so the descriptor stays 16-byte aligned).
pub const SA_HIDDEN: usize = 16;

/// What an element of an array of type `vt` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Element {
    /// Copied by its bytes.
    Bits,
    /// A `BSTR` the array owns.
    Bstr,
    /// An interface pointer the array holds a reference on.
    Interface,
    /// A `VARIANT` the array owns.
    Variant,
}

/// The size in bytes of an element of a `SafeArrayCreateVector(vt, ...)` array and what it is, or
/// `None` for a type that cannot be an element (`VT_EMPTY`, `VT_NULL`, the by-reference and array
/// types, records, anything unknown).
pub fn element(vt: u16) -> Option<(u32, Element)> {
    Some(match vt {
        // VT_I1, VT_UI1.
        16 | 17 => (1, Element::Bits),
        // VT_I2, VT_BOOL, VT_UI2.
        2 | 11 | 18 => (2, Element::Bits),
        // VT_I4, VT_R4, VT_ERROR, VT_UI4, VT_INT, VT_UINT.
        3 | 4 | 10 | 19 | 22 | 23 => (4, Element::Bits),
        // VT_R8, VT_CY, VT_DATE, VT_I8, VT_UI8.
        5 | 6 | 7 | 20 | 21 => (8, Element::Bits),
        // VT_DECIMAL.
        14 => (16, Element::Bits),
        VT_BSTR => (8, Element::Bstr),
        VT_UNKNOWN | VT_DISPATCH => (8, Element::Interface),
        VT_VARIANT => (crate::oleaut::VARIANT_SIZE as u32, Element::Variant),
        _ => return None,
    })
}

/// The `fFeatures` of a new array of type `vt`: the type is recorded, and the flag that says how
/// its elements are released.
pub fn features(vt: u16) -> u16 {
    FADF_HAVEVARTYPE
        | match vt {
            VT_BSTR => FADF_BSTR,
            VT_UNKNOWN => FADF_UNKNOWN,
            VT_DISPATCH => FADF_DISPATCH,
            VT_VARIANT => FADF_VARIANT,
            _ => 0,
        }
}

/// What the elements of an array with these features are (an array another module described:
/// its features say it).
pub fn element_of(features: u16) -> Element {
    if features & FADF_BSTR != 0 {
        Element::Bstr
    } else if features & (FADF_UNKNOWN | FADF_DISPATCH) != 0 {
        Element::Interface
    } else if features & FADF_VARIANT != 0 {
        Element::Variant
    } else {
        Element::Bits
    }
}

/// `SafeArrayGetVartype`: Learn: "If FADF_HAVEVARTYPE is set, returns the VARTYPE stored in the
/// array descriptor. If FADF_RECORD is set, it returns VT_RECORD; if FADF_DISPATCH is set, it
/// returns VT_DISPATCH; and if FADF_UNKNOWN is set, it returns VT_UNKNOWN." `stored` is the type
/// Poly keeps for an array it made. Poly also answers for the two other flags that name a type;
/// an array that says nothing about its type is `E_INVALIDARG`.
pub fn vartype(features: u16, stored: u16) -> Result<u16, u32> {
    if features & FADF_HAVEVARTYPE != 0 {
        Ok(stored)
    } else if features & FADF_RECORD != 0 {
        Ok(VT_RECORD)
    } else if features & FADF_DISPATCH != 0 {
        Ok(VT_DISPATCH)
    } else if features & FADF_UNKNOWN != 0 {
        Ok(VT_UNKNOWN)
    } else if features & FADF_BSTR != 0 {
        Ok(VT_BSTR)
    } else if features & FADF_VARIANT != 0 {
        Ok(VT_VARIANT)
    } else {
        Err(E_INVALIDARG)
    }
}

/// The bytes of the data of a vector of `count` elements of `size` bytes, or `None` when that
/// does not fit 32 bits (Learn: `SafeArrayCreateVector` returns NULL when the array cannot be
/// created).
pub fn data_size(size: u32, count: u32) -> Option<usize> {
    u32::try_from(u64::from(size) * u64::from(count)).ok().map(|n| n as usize)
}

/// `SafeArrayGetLBound` / `SafeArrayGetUBound(psa, nDim)`: the bounds of dimension `dim` (1 is the
/// first) of an array whose descriptor lists `bounds` as `(cElements, lLbound)`. Learn:
/// `DISP_E_BADINDEX` for a dimension the array does not have. The upper bound of an empty
/// dimension is one below the lower bound.
///
/// Learn, "SAFEARRAY structure": "The array rgsabound is stored with the left-most dimension in
/// rgsabound[0] and the right-most dimension in rgsabound[cDims - 1]", so dimension `dim` is
/// entry `dim - 1` (to be confirmed for arrays of several dimensions).
pub fn bound(bounds: &[(u32, i32)], dim: u32) -> Result<(i32, i32), u32> {
    if dim == 0 || dim as usize > bounds.len() {
        return Err(DISP_E_BADINDEX);
    }
    let (count, lower) = bounds[dim as usize - 1];
    Ok((lower, (i64::from(lower) + i64::from(count) - 1) as i32))
}

/// The position of the element `index` names in a vector with the bound `(cElements, lLbound)`:
/// `DISP_E_BADINDEX` outside it.
pub fn position(bound: (u32, i32), index: i32) -> Result<usize, u32> {
    let at = i64::from(index) - i64::from(bound.1);
    if at < 0 || at >= i64::from(bound.0) { Err(DISP_E_BADINDEX) } else { Ok(at as usize) }
}

/// `VARCMP_LT`, `VARCMP_EQ`, `VARCMP_GT` (`oleauto.h`): what `VarBstrCmp` returns, which is the
/// `CSTR_*` value of `CompareStringW` less one.
pub const VARCMP_LT: u32 = 0;
pub const VARCMP_EQ: u32 = 1;
pub const VARCMP_GT: u32 = 2;

/// `VarBstrCmp`'s result from the `CSTR_*` value `CompareStringW` gave (1, 2 or 3); a failed
/// comparison (0) is `E_INVALIDARG`.
pub fn varcmp(cstr: i32) -> u32 {
    match cstr {
        1 => VARCMP_LT,
        2 => VARCMP_EQ,
        3 => VARCMP_GT,
        _ => E_INVALIDARG,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_sizes_follow_the_variant_types() {
        assert_eq!(element(17), Some((1, Element::Bits)));
        assert_eq!(element(11), Some((2, Element::Bits)));
        assert_eq!(element(3), Some((4, Element::Bits)));
        assert_eq!(element(5), Some((8, Element::Bits)));
        assert_eq!(element(20), Some((8, Element::Bits)));
        assert_eq!(element(14), Some((16, Element::Bits)));
        assert_eq!(element(VT_BSTR), Some((8, Element::Bstr)));
        assert_eq!(element(VT_UNKNOWN), Some((8, Element::Interface)));
        assert_eq!(element(VT_DISPATCH), Some((8, Element::Interface)));
        assert_eq!(element(VT_VARIANT), Some((24, Element::Variant)));
        // VT_EMPTY, VT_NULL, VT_RECORD, VT_LPWSTR, an array of arrays.
        for vt in [0u16, 1, 36, 31, 0x2000 | 3] {
            assert_eq!(element(vt), None, "{vt}");
        }
    }

    #[test]
    fn features_name_the_type_and_how_elements_are_released() {
        assert_eq!(features(3), FADF_HAVEVARTYPE);
        assert_eq!(features(VT_BSTR), FADF_HAVEVARTYPE | FADF_BSTR);
        assert_eq!(features(VT_VARIANT), FADF_HAVEVARTYPE | FADF_VARIANT);
        assert_eq!(element_of(features(VT_BSTR)), Element::Bstr);
        assert_eq!(element_of(features(VT_DISPATCH)), Element::Interface);
        assert_eq!(element_of(features(VT_VARIANT)), Element::Variant);
        assert_eq!(element_of(features(17)), Element::Bits);
        assert_eq!(vartype(features(17), 17), Ok(17));
        assert_eq!(vartype(FADF_RECORD, 0), Ok(VT_RECORD));
        assert_eq!(vartype(FADF_DISPATCH, 0), Ok(VT_DISPATCH));
        assert_eq!(vartype(FADF_UNKNOWN, 0), Ok(VT_UNKNOWN));
        assert_eq!(vartype(FADF_BSTR, 0), Ok(VT_BSTR));
        assert_eq!(vartype(0, 3), Err(E_INVALIDARG));
    }

    #[test]
    fn bounds_and_positions() {
        // A vector of 5 elements from index 10.
        assert_eq!(bound(&[(5, 10)], 1), Ok((10, 14)));
        assert_eq!(bound(&[(5, 10)], 0), Err(DISP_E_BADINDEX));
        assert_eq!(bound(&[(5, 10)], 2), Err(DISP_E_BADINDEX));
        // An empty vector: the upper bound is below the lower one.
        assert_eq!(bound(&[(0, 0)], 1), Ok((0, -1)));
        // Two dimensions, [2][5] in Learn's example: the first is entry 0.
        assert_eq!(bound(&[(2, 0), (5, 0)], 1), Ok((0, 1)));
        assert_eq!(bound(&[(2, 0), (5, 0)], 2), Ok((0, 4)));
        assert_eq!(position((5, 10), 10), Ok(0));
        assert_eq!(position((5, 10), 14), Ok(4));
        assert_eq!(position((5, 10), 9), Err(DISP_E_BADINDEX));
        assert_eq!(position((5, 10), 15), Err(DISP_E_BADINDEX));
        assert_eq!(position((0, 0), 0), Err(DISP_E_BADINDEX));
        // The widest vector: indexes from i32::MIN, the last one is i32::MAX - 1.
        assert_eq!(position((u32::MAX, i32::MIN), i32::MAX - 1), Ok(u32::MAX as usize - 1));
        assert_eq!(position((u32::MAX, i32::MIN), i32::MAX), Err(DISP_E_BADINDEX));
    }

    #[test]
    fn data_sizes_and_comparison_results() {
        assert_eq!(data_size(8, 4), Some(32));
        assert_eq!(data_size(24, 0), Some(0));
        assert_eq!(data_size(24, u32::MAX), None);
        assert_eq!((varcmp(1), varcmp(2), varcmp(3)), (VARCMP_LT, VARCMP_EQ, VARCMP_GT));
        assert_eq!(varcmp(0), E_INVALIDARG);
    }
}
