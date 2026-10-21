//! Which functions `advapi32.dll` forwards to `kernelbase`, and where (ADR
//! 0083), like `kernel32`'s ([`crate::exports`], ADR 0079).
//!
//! Learn's reference pages name the DLL `Advapi32.dll`; their metadata
//! (the "APILocation" list of each page) also names the API-set contracts
//! and `KernelBase.dll` for the registry functions
//! (`api-ms-win-core-registry-l1-1-0`), the token functions
//! (`api-ms-win-core-processthreads-l1-1-0`) and the SID, ACL and security
//! descriptor functions (`api-ms-win-security-base-l1-1-0`). Our
//! `advapi32.dll` forwards each of those to `<contract>.<name>`, which the
//! loader resolves to `kernelbase.dll` (`poly_ldr::apiset`).
//!
//! [`ADVAPI32_OWN`] are the functions whose locations name no
//! `kernelbase` contract: `RegDeleteKeyW/A` (`api-ms-win-core-registry-l2`,
//! hosted by `advapi32` itself), the SID string conversions
//! (`api-ms-win-security-sddl-l1-1-0`, which Poly's API-set table gives to
//! `advapi32`), `GetUserNameW/A` and the privilege-name lookups
//! (`api-ms-win-security-lsalookup`, also `advapi32`'s). `advapi32`
//! implements them on top of `kernelbase`.
//!
//! The contracts are Poly's reading of Learn; a real `advapi32.dll`'s
//! forwarder strings are export-table data the spec agent may compare (to
//! be confirmed).

const REGISTRY: &str = "api-ms-win-core-registry-l1-1-0";
const PROCESS_THREADS: &str = "api-ms-win-core-processthreads-l1-1-0";
const SECURITY_BASE: &str = "api-ms-win-security-base-l1-1-0";

/// `(function, contract)`: `advapi32!function` forwards to
/// `contract.function`.
pub const ADVAPI32_FORWARDS: &[(&str, &str)] = &[
    // The registry (winreg.h).
    ("RegOpenKeyExW", REGISTRY),
    ("RegOpenKeyExA", REGISTRY),
    ("RegCreateKeyExW", REGISTRY),
    ("RegCreateKeyExA", REGISTRY),
    ("RegQueryValueExW", REGISTRY),
    ("RegQueryValueExA", REGISTRY),
    ("RegSetValueExW", REGISTRY),
    ("RegSetValueExA", REGISTRY),
    ("RegGetValueW", REGISTRY),
    ("RegGetValueA", REGISTRY),
    ("RegEnumKeyExW", REGISTRY),
    ("RegEnumValueW", REGISTRY),
    ("RegDeleteKeyExW", REGISTRY),
    ("RegDeleteKeyExA", REGISTRY),
    ("RegDeleteValueW", REGISTRY),
    ("RegDeleteValueA", REGISTRY),
    ("RegCloseKey", REGISTRY),
    ("RegFlushKey", REGISTRY),
    ("RegQueryInfoKeyW", REGISTRY),
    // Tokens (processthreadsapi.h, securitybaseapi.h).
    ("OpenProcessToken", PROCESS_THREADS),
    ("OpenThreadToken", PROCESS_THREADS),
    ("GetTokenInformation", SECURITY_BASE),
    ("CheckTokenMembership", SECURITY_BASE),
    // SIDs.
    ("AllocateAndInitializeSid", SECURITY_BASE),
    ("FreeSid", SECURITY_BASE),
    ("EqualSid", SECURITY_BASE),
    ("EqualPrefixSid", SECURITY_BASE),
    ("GetLengthSid", SECURITY_BASE),
    ("IsValidSid", SECURITY_BASE),
    ("CopySid", SECURITY_BASE),
    ("InitializeSid", SECURITY_BASE),
    ("GetSidLengthRequired", SECURITY_BASE),
    ("GetSidIdentifierAuthority", SECURITY_BASE),
    ("GetSidSubAuthority", SECURITY_BASE),
    ("GetSidSubAuthorityCount", SECURITY_BASE),
    // Security descriptors.
    ("InitializeSecurityDescriptor", SECURITY_BASE),
    ("IsValidSecurityDescriptor", SECURITY_BASE),
    ("GetSecurityDescriptorLength", SECURITY_BASE),
    ("GetSecurityDescriptorControl", SECURITY_BASE),
    ("SetSecurityDescriptorControl", SECURITY_BASE),
    ("SetSecurityDescriptorDacl", SECURITY_BASE),
    ("GetSecurityDescriptorDacl", SECURITY_BASE),
    ("SetSecurityDescriptorSacl", SECURITY_BASE),
    ("GetSecurityDescriptorSacl", SECURITY_BASE),
    ("SetSecurityDescriptorOwner", SECURITY_BASE),
    ("GetSecurityDescriptorOwner", SECURITY_BASE),
    ("SetSecurityDescriptorGroup", SECURITY_BASE),
    ("GetSecurityDescriptorGroup", SECURITY_BASE),
    ("MakeSelfRelativeSD", SECURITY_BASE),
    // ACLs.
    ("InitializeAcl", SECURITY_BASE),
    ("IsValidAcl", SECURITY_BASE),
    ("GetAclInformation", SECURITY_BASE),
    ("AddAccessAllowedAce", SECURITY_BASE),
    ("AddAccessAllowedAceEx", SECURITY_BASE),
    ("AddAccessDeniedAce", SECURITY_BASE),
    ("AddAccessDeniedAceEx", SECURITY_BASE),
    ("AddAce", SECURITY_BASE),
    ("GetAce", SECURITY_BASE),
    ("DeleteAce", SECURITY_BASE),
    ("FindFirstFreeAce", SECURITY_BASE),
];

/// The functions `advapi32.dll` implements itself.
pub const ADVAPI32_OWN: &[&str] = &[
    "RegDeleteKeyW",
    "RegDeleteKeyA",
    "ConvertSidToStringSidW",
    "ConvertSidToStringSidA",
    "ConvertStringSidToSidW",
    "ConvertStringSidToSidA",
    "GetUserNameW",
    "GetUserNameA",
    "LookupPrivilegeValueW",
    "LookupPrivilegeNameW",
];

/// The forwarder string of `advapi32!name` (PE format: `DLLName.FunctionName`).
pub fn forwarder(name: &str) -> Option<alloc::string::String> {
    ADVAPI32_FORWARDS.iter().find(|&&(n, _)| n == name).map(|&(n, c)| alloc::format!("{c}.{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use poly_ldr::apiset::{ApiSet, resolve};
    use std::collections::BTreeSet;

    #[test]
    fn forwarded_contracts_are_kernelbases_and_own_ones_are_advapis() {
        for &(name, contract) in ADVAPI32_FORWARDS {
            let w: std::vec::Vec<u16> = contract.encode_utf16().collect();
            assert_eq!(resolve(&w), ApiSet::Host("kernelbase.dll"), "{name} -> {contract}");
        }
        // The SDDL contract the SID string functions belong to is advapi32's.
        let sddl: std::vec::Vec<u16> = "api-ms-win-security-sddl-l1-1-0".encode_utf16().collect();
        assert_eq!(resolve(&sddl), ApiSet::Host("advapi32.dll"));
    }

    #[test]
    fn names_are_unique_and_kernelbase_exports_them() {
        let mut seen = BTreeSet::new();
        for name in ADVAPI32_FORWARDS.iter().map(|&(n, _)| n).chain(ADVAPI32_OWN.iter().copied()) {
            assert!(seen.insert(name), "{name} listed twice");
        }
        let kb: BTreeSet<&str> = crate::exports::kernelbase_exports().collect();
        for &(name, _) in ADVAPI32_FORWARDS {
            assert!(kb.contains(name), "{name}");
        }
        assert_eq!(forwarder("RegCloseKey").as_deref(), Some("api-ms-win-core-registry-l1-1-0.RegCloseKey"));
        assert_eq!(forwarder("GetUserNameW"), None);
    }
}
