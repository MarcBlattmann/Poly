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

pub mod access;
pub mod account;
pub mod acl;
pub mod condace;
pub mod err;
pub mod error;
pub mod explicit;
pub mod inherit;
pub mod sd;
pub mod sddl;
pub mod sdops;
pub mod sdstr;
#[cfg(test)]
mod sdstr_sheet;
pub mod sid;
pub mod status;
pub mod token;
pub mod wellknown;

pub use sid::Sid;
pub use status::NtStatus;

/// Poly's single user's name (the default hives' `POLY_USER_NAME`, ADR 0066).
pub const POLY_USER_NAME: &str = "poly";
/// The machine's account domain: `S-1-5-21-520193847-884726105-1936024571` (three large
/// sub-authorities like a real machine's; their digits (9, 9 and 10) are what
/// the string lengths of the sheets imply, ADR 0171); local accounts are
/// relative to it.
pub const MACHINE_DOMAIN: [u32; 4] = [21, 520_193_847, 884_726_105, 1_936_024_571];
/// Poly's single user's relative identifier (the default hives'
/// `S-1-5-21-520193847-884726105-1936024571-1002`).
pub const POLY_USER_RID: u32 = 1002;

/// Poly's single user's SID, `S-1-5-21-520193847-884726105-1936024571-1002`.
pub fn poly_user_sid() -> Sid {
    sid::domain_relative(POLY_USER_RID)
}

/// The registry path of the current user's hive, `\REGISTRY\USER\<the user's SID>`: what
/// `RtlFormatCurrentUserKeyPath` returns (programs build `HKEY_CURRENT_USER` paths for the
/// native registry functions from it).
pub fn current_user_key_path() -> alloc::string::String {
    alloc::format!("\\REGISTRY\\USER\\{}", sddl::to_string(&poly_user_sid()))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use alloc::string::ToString;

    /// The path names the hive the default registry really has (it used to be a literal in
    /// ntdll that kept the SID from before ADR 0171, so `NtCreateKey` below it failed).
    #[test]
    fn the_current_user_key_path_is_the_default_user_hive() {
        assert_eq!(
            current_user_key_path(),
            alloc::format!("\\REGISTRY\\USER\\{}", poly_registry::defaults::POLY_USER_SID)
        );
        assert_eq!(current_user_key_path(), "\\REGISTRY\\USER\\S-1-5-21-520193847-884726105-1936024571-1002");
    }

    #[test]
    fn the_user_matches_the_default_hives() {
        assert_eq!(sddl::to_string(&poly_user_sid()), poly_registry::defaults::POLY_USER_SID.to_string());
        assert_eq!(POLY_USER_NAME, poly_registry::defaults::POLY_USER_NAME);
    }
}
