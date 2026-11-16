//! `D3D11CreateDevice`'s arguments (D3D11CreateDevice.yaml): the driver type, the flags, the
//! feature level list and the SDK version decide whether a device is made and at which level.

use crate::hr::*;

/// `D3D_DRIVER_TYPE` (d3dcommon.h).
pub mod driver {
    pub const UNKNOWN: u32 = 0;
    pub const HARDWARE: u32 = 1;
    pub const REFERENCE: u32 = 2;
    pub const NULL: u32 = 3;
    pub const SOFTWARE: u32 = 4;
    pub const WARP: u32 = 5;
}

/// `D3D_FEATURE_LEVEL` values the software device supports, 9_1 to 12_1 (as WARP).
pub const LEVELS: [u32; 9] = [0x9100, 0x9200, 0x9300, 0xA000, 0xA100, 0xB000, 0xB100, 0xC000, 0xC100];
/// The level of a device made without a list.
pub const DEFAULT_LEVEL: u32 = 0xB000;
/// `D3D11_SDK_VERSION` (d3d11.h).
pub const SDK_VERSION: u32 = 7;

/// `D3D11_CREATE_DEVICE_FLAG` (d3d11.h).
pub mod flag {
    pub const SINGLETHREADED: u32 = 1;
    pub const DEBUG: u32 = 2;
    pub const SWITCH_TO_REF: u32 = 4;
    pub const BGRA_SUPPORT: u32 = 0x20;
    pub const VIDEO_SUPPORT: u32 = 0x800;
}

/// What the caller passed.
#[derive(Clone, Copy, Debug)]
pub struct Args<'a> {
    pub driver: u32,
    /// A non-NULL `pAdapter`.
    pub adapter: bool,
    /// A non-NULL `Software` module.
    pub software: bool,
    pub flags: u32,
    /// `pFeatureLevels` (None for NULL) and `FeatureLevels`.
    pub levels: Option<&'a [u32]>,
    pub count: u32,
    pub sdk: u32,
}

/// The feature level of the device these arguments make, or the failure. Poly is a machine whose
/// only adapter is the software one and without the SDK layers (the oracle's results are the
/// reference for the flags that need them).
pub fn level(a: &Args) -> Result<u32, HRESULT> {
    if a.sdk != SDK_VERSION {
        return Err(E_INVALIDARG);
    }
    match (a.driver, a.adapter) {
        // UNKNOWN takes the adapter it is given; the others make their own.
        (driver::UNKNOWN, true) => {}
        (driver::HARDWARE, false) | (driver::WARP, false) | (driver::NULL, false) => {}
        (driver::REFERENCE, false) => return Err(DXGI_ERROR_UNSUPPORTED),
        (driver::SOFTWARE, false) if a.software => return Err(DXGI_ERROR_UNSUPPORTED),
        _ => return Err(E_INVALIDARG),
    }
    // Bits that no flag uses in the middle of the word make the call invalid (0xFFFFFFFF); the
    // undefined top bit is accepted and kept.
    if a.flags & 0x7FFF_0000 != 0 {
        return Err(E_INVALIDARG);
    }
    if a.flags & flag::VIDEO_SUPPORT != 0 {
        return Err(DXGI_ERROR_UNSUPPORTED);
    }
    // The debug layers and the reference switch are optional Windows components Poly does not have.
    if a.flags & (flag::DEBUG | flag::SWITCH_TO_REF | 0x10) != 0 {
        return Err(DXGI_ERROR_SDK_COMPONENT_MISSING);
    }
    match a.levels {
        None if a.count != 0 => Err(E_INVALIDARG),
        None => Ok(DEFAULT_LEVEL),
        Some(_) if a.count == 0 => Err(DXGI_ERROR_UNSUPPORTED),
        Some(list) => {
            if list.iter().any(|l| !LEVELS.contains(l)) {
                return Err(E_INVALIDARG);
            }
            // The first entry in list order (every known level is supported).
            Ok(list[0])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(driver: u32) -> Args<'static> {
        Args { driver, adapter: false, software: false, flags: 0, levels: None, count: 0, sdk: 7 }
    }

    #[test]
    fn driver_types() {
        assert_eq!(level(&args(driver::WARP)), Ok(0xB000));
        assert_eq!(level(&args(driver::HARDWARE)), Ok(0xB000));
        assert_eq!(level(&args(driver::NULL)), Ok(0xB000));
        assert_eq!(level(&args(driver::UNKNOWN)), Err(E_INVALIDARG));
        assert_eq!(level(&Args { adapter: true, ..args(driver::UNKNOWN) }), Ok(0xB000));
        assert_eq!(level(&Args { adapter: true, ..args(driver::WARP) }), Err(E_INVALIDARG));
        assert_eq!(level(&Args { adapter: true, ..args(driver::HARDWARE) }), Err(E_INVALIDARG));
        assert_eq!(level(&args(driver::REFERENCE)), Err(DXGI_ERROR_UNSUPPORTED));
        assert_eq!(level(&args(driver::SOFTWARE)), Err(E_INVALIDARG));
        assert_eq!(level(&args(6)), Err(E_INVALIDARG));
        assert_eq!(level(&args(99)), Err(E_INVALIDARG));
    }

    #[test]
    fn flags() {
        let f = |flags| level(&Args { flags, ..args(driver::WARP) });
        for ok in [0, 1, 0x20, 8, 0x21, 0x8000_0000] {
            assert_eq!(f(ok), Ok(0xB000), "{ok:#x}");
        }
        assert_eq!(f(0x800), Err(DXGI_ERROR_UNSUPPORTED));
        assert_eq!(f(0x821), Err(DXGI_ERROR_UNSUPPORTED));
        assert_eq!(f(0x4), Err(DXGI_ERROR_SDK_COMPONENT_MISSING));
        assert_eq!(f(0x10), Err(DXGI_ERROR_SDK_COMPONENT_MISSING));
        assert_eq!(f(0xFFFF_FFFF), Err(E_INVALIDARG));
    }

    #[test]
    fn sdk_versions() {
        for v in [0, 1, 6, 8, 0x1000_0000, 0xFFFF_FFFF] {
            assert_eq!(level(&Args { sdk: v, ..args(driver::WARP) }), Err(E_INVALIDARG));
        }
    }

    #[test]
    fn level_lists() {
        let f = |l: &'static [u32]| level(&Args { levels: Some(l), count: l.len() as u32, ..args(driver::WARP) });
        assert_eq!(f(&[0xB100]), Ok(0xB100));
        assert_eq!(f(&[0xA000, 0xB000]), Ok(0xA000));
        assert_eq!(f(&[0x9100, 0xB000]), Ok(0x9100));
        assert_eq!(f(&[0xC100, 0xB000]), Ok(0xC100));
        assert_eq!(f(&[0xB000, 0xB000]), Ok(0xB000));
        assert_eq!(f(&[0]), Err(E_INVALIDARG));
        assert_eq!(f(&[0xB000, 0x1234]), Err(E_INVALIDARG));
        assert_eq!(level(&Args { levels: None, count: 3, ..args(driver::WARP) }), Err(E_INVALIDARG));
        assert_eq!(
            level(&Args { levels: Some(&[0xB000]), count: 0, ..args(driver::WARP) }),
            Err(DXGI_ERROR_UNSUPPORTED)
        );
    }
}
