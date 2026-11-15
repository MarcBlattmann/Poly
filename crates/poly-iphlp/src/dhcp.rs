//! `DhcpRequestParams`' argument rules (`dhcpcsvc.dll`, ADR 0155): which requests are refused and
//! with what. Poly's adapters are configured by the boot setting, so a request for a real adapter
//! succeeds with no data; one for an adapter that does not exist fails as on Windows.
//!
//! Sources: Learn's "DhcpRequestParams function (dhcpcsdk.h)", "DhcpCApiInitialize"; `dhcpcsdk.h`
//! (`DHCPCAPI_REQUEST_*`), `winerror.h`; and the observations of
//! `specs/windows/dhcpcsvc.dll/DhcpCApiInitialize.yaml`.

use crate::{ERROR_INVALID_PARAMETER, IfInfo};

/// `ERROR_INVALID_NAME` (`winerror.h`).
pub const ERROR_INVALID_NAME: u32 = 123;

/// `DHCPCAPI_REQUEST_*` flags (`dhcpcsdk.h`): persistent, synchronous, asynchronous, cancel.
pub const DHCPCAPI_REQUEST_MASK: u32 = 0x0F;

/// The result of `DhcpRequestParams`: `Ok(())` when the request is valid for one of `adapters`
/// (no option data is ever received), else the error. Before `DhcpCApiInitialize` (or after
/// `DhcpCApiCleanup`), with flags outside the mask or without an adapter name: 87. An adapter that
/// does not exist: 87, or 123 when the call asks for parameters (`receive_count`, the receive
/// array's `nParams`).
pub fn request_params(
    initialized: bool,
    flags: u32,
    adapter: Option<&str>,
    receive_count: u32,
    adapters: &[IfInfo],
) -> Result<(), u32> {
    if !initialized || flags & !DHCPCAPI_REQUEST_MASK != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let Some(name) = adapter else { return Err(ERROR_INVALID_PARAMETER) };
    if adapters.iter().any(|a| a.adapter_name().eq_ignore_ascii_case(name)) {
        Ok(())
    } else if receive_count > 0 {
        Err(ERROR_INVALID_NAME)
    } else {
        Err(ERROR_INVALID_PARAMETER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn rules() {
        let ifs = vec![IfInfo::loopback()];
        let name = ifs[0].adapter_name();
        assert_eq!(request_params(false, 2, Some(&name), 1, &ifs), Err(87));
        assert_eq!(request_params(true, 2, Some(&name), 1, &ifs), Ok(()));
        assert_eq!(request_params(true, 2, Some(&name.to_lowercase()), 0, &ifs), Ok(()));
        assert_eq!(request_params(true, 0xFFFF_0000, Some(&name), 0, &ifs), Err(87));
        assert_eq!(request_params(true, 2, None, 1, &ifs), Err(87));
        assert_eq!(request_params(true, 2, Some("nope"), 0, &ifs), Err(87));
        assert_eq!(request_params(true, 2, Some(""), 0, &ifs), Err(87));
        assert_eq!(request_params(true, 2, Some("nope"), 1, &ifs), Err(123));
    }
}
