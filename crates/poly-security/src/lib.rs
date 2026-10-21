//! Windows security data for Poly's single user (ADR 0083), as host-tested
//! code shared by the kernel (the token services) and our `kernelbase` /
//! `advapi32` (the Win32 security functions).
//!
//! Poly has one user and enforces nothing beyond handle access masks (ADR
//! 0066): security descriptors and ACLs are *stored and returned*, never
//! evaluated. What has to be exact is their binary layout, which programs
//! build and walk themselves:
//!
//! - [`sid`]: `SID` (`winnt.h`): its layout, validity, the
//!   well-known identifiers and authorities Poly uses.
//! - [`sddl`]: SID strings: `S-R-I-S-S...` and the two-letter SID string
//!   constants (Learn: "SID Strings", "SID Components").
//! - [`acl`]: `ACL` and `ACE_HEADER`-led ACEs (`winnt.h`): initialise,
//!   validate, add, insert, delete and walk ACEs, `ACL_*_INFORMATION`.
//! - [`sd`]: `SECURITY_DESCRIPTOR` (absolute, x64 layout) and
//!   `SECURITY_DESCRIPTOR_RELATIVE`, their control bits and conversion.
//! - [`token`]: the single user's access token and the
//!   `TOKEN_INFORMATION_CLASS` structures `NtQueryInformationToken`
//!   returns, laid out at the caller's buffer address.
//!
//! Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's reference pages of
//! each function and structure named, "SID Strings", "Well-known SIDs",
//! "Access Rights for Access-Token Objects", "Mandatory Integrity Control",
//! and the constants and layouts of the public SDK headers (`winnt.h`,
//! `winerror.h`, `ntstatus.h`, `sddl.h`, `wdm.h` for the privilege LUIDs).
//! Where they leave a behaviour open the code says what it picks ("to be
//! confirmed by the spec agent", ADR 0083).

#![no_std]

extern crate alloc;

pub mod acl;
pub mod error;
pub mod sd;
pub mod sddl;
pub mod sid;
pub mod token;

pub use poly_ntobj::NtStatus;
pub use sid::Sid;

/// Poly's single user's name (the default hives' `POLY_USER_NAME`, ADR 0066).
pub const POLY_USER_NAME: &str = "poly";
/// The machine's account domain: `S-1-5-21-0-0-0` (three zero
/// sub-authorities, ADR 0066); local accounts are relative to it.
pub const MACHINE_DOMAIN: [u32; 4] = [21, 0, 0, 0];
/// Poly's single user's relative identifier (the default hives'
/// `S-1-5-21-0-0-0-1000`).
pub const POLY_USER_RID: u32 = 1000;

/// Poly's single user's SID, `S-1-5-21-0-0-0-1000`.
pub fn poly_user_sid() -> Sid {
    sid::domain_relative(POLY_USER_RID)
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use alloc::string::ToString;

    #[test]
    fn the_user_matches_the_default_hives() {
        assert_eq!(sddl::to_string(&poly_user_sid()), poly_registry::defaults::POLY_USER_SID.to_string());
        assert_eq!(POLY_USER_NAME, poly_registry::defaults::POLY_USER_NAME);
    }
}
