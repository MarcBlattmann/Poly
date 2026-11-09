//! `HRESULT`s (`winerror.h`).

pub type Hresult = i32;

pub const S_OK: Hresult = 0;
pub const S_FALSE: Hresult = 1;
pub const E_NOTIMPL: Hresult = 0x8000_4001_u32 as i32;
pub const E_NOINTERFACE: Hresult = 0x8000_4002_u32 as i32;
pub const E_POINTER: Hresult = 0x8000_4003_u32 as i32;
pub const E_FAIL: Hresult = 0x8000_4005_u32 as i32;
pub const E_UNEXPECTED: Hresult = 0x8000_FFFF_u32 as i32;
pub const E_OUTOFMEMORY: Hresult = 0x8007_000E_u32 as i32;
pub const E_INVALIDARG: Hresult = 0x8007_0057_u32 as i32;
/// `RPC_E_CHANGED_MODE`: the thread already has another concurrency model.
pub const RPC_E_CHANGED_MODE: Hresult = 0x8001_0106_u32 as i32;
/// `CO_E_NOTINITIALIZED`.
pub const CO_E_NOTINITIALIZED: Hresult = 0x8004_01F0_u32 as i32;
/// `REGDB_E_CLASSNOTREG`: the class is not registered.
pub const REGDB_E_CLASSNOTREG: Hresult = 0x8004_0154_u32 as i32;
/// `CO_E_CLASSSTRING`: the class string is not valid.
pub const CO_E_CLASSSTRING: Hresult = 0x8004_01F3_u32 as i32;
/// `CO_E_IIDSTRING`.
pub const CO_E_IIDSTRING: Hresult = 0x8004_0161_u32 as i32;
/// `CLASS_E_NOAGGREGATION`.
pub const CLASS_E_NOAGGREGATION: Hresult = 0x8004_0110_u32 as i32;
/// `DRAGDROP_E_NOTREGISTERED`, `DRAGDROP_E_ALREADYREGISTERED`, `DRAGDROP_E_INVALIDHWND`.
pub const DRAGDROP_E_NOTREGISTERED: Hresult = 0x8004_0100_u32 as i32;
pub const DRAGDROP_E_ALREADYREGISTERED: Hresult = 0x8004_0101_u32 as i32;
pub const DRAGDROP_E_INVALIDHWND: Hresult = 0x8004_0102_u32 as i32;
/// `RPC_E_TOO_LATE`: `CoInitializeSecurity` after the first marshalling.
pub const RPC_E_TOO_LATE: Hresult = 0x8001_0119_u32 as i32;

/// `HRESULT_FROM_WIN32` (`winerror.h`).
pub const fn from_win32(code: u32) -> Hresult {
    if code == 0 { 0 } else { ((code & 0xFFFF) | 0x8007_0000) as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_win32_sets_the_facility() {
        assert_eq!(from_win32(0), S_OK);
        assert_eq!(from_win32(87), E_INVALIDARG);
        assert_eq!(from_win32(14), E_OUTOFMEMORY);
    }
}
