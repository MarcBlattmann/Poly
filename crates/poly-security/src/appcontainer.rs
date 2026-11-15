//! AppContainers: the SIDs of a name, the capability SIDs, the rules for profile names and the
//! text of an object directory (ADR 0196). What the sheets of `spec-chrome-win-s2b` fix:
//!
//! - `DeriveAppContainerSidFromAppContainerName`: `S-1-15-2-` and the first seven little-endian
//!   words of the SHA-256 of the lower-cased name in UTF-16LE.
//! - `DeriveCapabilitySidsFromName`: the first eight words of the SHA-256 of the *upper-cased*
//!   name give the group SID `S-1-5-32-<8 words>` and, unless the name is a well-known
//!   capability (`S-1-15-3-<rid>`), the capability SID `S-1-15-3-1024-<8 words>`.
//! - `CreateAppContainerProfile` names: 1 to 64 units, no `\` or `/` (`ERROR_PATH_NOT_FOUND`),
//!   no `:` (`ERROR_DIRECTORY`), a display name below 512 and a description below 2048 units.
//!
//! Sources: the sheets, Learn ("Implementing an AppContainer", `userenv.h`,
//! `securitybaseapi.h`, `winnt.h` for `SECURITY_CAPABILITIES`).

use crate::sddl;
use crate::sid::Sid;
use alloc::string::String;
use poly_crypto::sha256::Sha256;

/// `SECURITY_APP_PACKAGE_AUTHORITY` (`winnt.h`): `S-1-15`.
pub const APP_PACKAGE_AUTHORITY: u64 = 15;
/// `SECURITY_APP_PACKAGE_BASE_RID` and `SECURITY_CAPABILITY_BASE_RID` (`winnt.h`).
pub const APP_PACKAGE_BASE_RID: u32 = 2;
pub const CAPABILITY_BASE_RID: u32 = 3;
/// The first sub-authority after `S-1-15-3` of a capability derived from a name.
pub const CAPABILITY_HASHED_RID: u32 = 1024;
/// A package SID has 8 sub-authorities (`SECURITY_BUILTIN_PACKAGE_ANY_PACKAGE`'s family).
pub const PACKAGE_SUB_AUTHORITIES: usize = 8;

/// `HRESULT`s of `winerror.h` the functions answer.
pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_ALREADY_EXISTS: u32 = 0x8007_00B7;
pub const E_FILE_NOT_FOUND: u32 = 0x8007_0002;
pub const E_PATH_NOT_FOUND: u32 = 0x8007_0003;
pub const E_DIRECTORY: u32 = 0x8007_010B;
pub const E_NOT_APPCONTAINER: u32 = 0x8007_109A;
pub const E_INVALID_SID: u32 = 0x8007_0539;
pub const E_UNEXPECTED: u32 = 0x8000_FFFF;
/// `HRESULT_FROM_WIN32(ERROR_INVALID_DATA)`-like answer of `AppContainerLookupMoniker` for a
/// moniker longer than 64 units (the sheet: `0x80072023`).
pub const E_MONIKER_TOO_LONG: u32 = 0x8007_2023;

/// The longest profile name and moniker (units).
pub const MAX_NAME: usize = 64;
/// Display names stay below this many units, descriptions below `MAX_DESCRIPTION`.
pub const MAX_DISPLAY_NAME: usize = 512;
pub const MAX_DESCRIPTION: usize = 2048;

/// `ALL APPLICATION PACKAGES`, `S-1-15-2-1`.
pub fn all_application_packages() -> Sid {
    Sid::new(APP_PACKAGE_AUTHORITY, &[APP_PACKAGE_BASE_RID, 1])
}

/// `ALL RESTRICTED APPLICATION PACKAGES`, `S-1-15-2-2`.
pub fn all_restricted_application_packages() -> Sid {
    Sid::new(APP_PACKAGE_AUTHORITY, &[APP_PACKAGE_BASE_RID, 2])
}

fn lower(u: u16) -> u16 {
    char::from_u32(u32::from(u))
        .and_then(|c| c.to_lowercase().next())
        .map_or(u, |c| if (c as u32) <= 0xFFFF { c as u32 as u16 } else { u })
}

fn upper(u: u16) -> u16 {
    char::from_u32(u32::from(u))
        .and_then(|c| c.to_uppercase().next())
        .map_or(u, |c| if (c as u32) <= 0xFFFF { c as u32 as u16 } else { u })
}

fn words_of(name: &[u16], fold: fn(u16) -> u16) -> [u32; 8] {
    let mut h = Sha256::new();
    for &u in name {
        h.update(&fold(u).to_le_bytes());
    }
    let d = h.finish();
    let mut w = [0u32; 8];
    for (i, v) in w.iter_mut().enumerate() {
        *v = u32::from_le_bytes([d[4 * i], d[4 * i + 1], d[4 * i + 2], d[4 * i + 3]]);
    }
    w
}

/// The package SID of a profile name (any length: the caller checks 1..=64).
pub fn package_sid(name: &[u16]) -> Sid {
    let w = words_of(name, lower);
    let mut subs = [0u32; 8];
    subs[0] = APP_PACKAGE_BASE_RID;
    subs[1..].copy_from_slice(&w[..7]);
    Sid::new(APP_PACKAGE_AUTHORITY, &subs)
}

/// Whether `s` is a package SID: `S-1-15-2` with exactly eight sub-authorities.
pub fn is_package_sid(s: &Sid) -> bool {
    s.authority == APP_PACKAGE_AUTHORITY && s.subs.len() == PACKAGE_SUB_AUTHORITIES && s.subs[0] == APP_PACKAGE_BASE_RID
}

/// Whether `s` is a capability SID: `S-1-15-3-...` with at least one more sub-authority.
pub fn is_capability_sid(s: &Sid) -> bool {
    s.authority == APP_PACKAGE_AUTHORITY && s.subs.len() >= 2 && s.subs[0] == CAPABILITY_BASE_RID
}

/// The well-known capabilities the sheet lists, by their relative identifier (case-insensitive).
const WELL_KNOWN: [(&str, u32); 12] = [
    ("internetClient", 1),
    ("internetClientServer", 2),
    ("privateNetworkClientServer", 3),
    ("picturesLibrary", 4),
    ("videosLibrary", 5),
    ("musicLibrary", 6),
    ("documentsLibrary", 7),
    ("enterpriseAuthentication", 8),
    ("sharedUserCertificates", 9),
    ("removableStorage", 10),
    ("appointments", 11),
    ("contacts", 12),
];

fn well_known(name: &[u16]) -> Option<u32> {
    WELL_KNOWN.iter().find_map(|&(n, rid)| {
        let same = n.len() == name.len() && n.bytes().zip(name).all(|(a, &b)| upper(u16::from(a)) == upper(b));
        same.then_some(rid)
    })
}

/// `DeriveCapabilitySidsFromName`: the group SID and the capability SID of a name.
pub fn capability_sids(name: &[u16]) -> (Sid, Sid) {
    let w = words_of(name, upper);
    let mut group = [0u32; 9];
    group[0] = 32;
    group[1..].copy_from_slice(&w);
    let capability = match well_known(name) {
        Some(rid) => Sid::new(APP_PACKAGE_AUTHORITY, &[CAPABILITY_BASE_RID, rid]),
        None => {
            let mut subs = [0u32; 10];
            subs[0] = CAPABILITY_BASE_RID;
            subs[1] = CAPABILITY_HASHED_RID;
            subs[2..].copy_from_slice(&w);
            Sid::new(APP_PACKAGE_AUTHORITY, &subs)
        }
    };
    (Sid::new(5, &group), capability)
}

/// `GetAppContainerNamedObjectPath`: the object-manager directory of a container's named
/// objects (session 0: no `\Sessions\n` prefix, the sheet).
pub fn named_object_path(container: &Sid) -> String {
    alloc::format!("AppContainerNamedObjects\\{}", sddl::to_string(container))
}

/// The answer of `CreateAppContainerProfile` (and `DeleteAppContainerProfile`) to a name: the
/// units checked in the order of the sheet (empty and too long: `E_INVALIDARG`, then a slash:
/// path not found, then a colon: `ERROR_DIRECTORY`).
pub fn check_profile_name(name: &[u16]) -> Result<(), u32> {
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(E_INVALIDARG);
    }
    if name.iter().any(|&u| u == u16::from(b'\\') || u == u16::from(b'/')) {
        return Err(E_PATH_NOT_FOUND);
    }
    if name.contains(&u16::from(b':')) {
        return Err(E_DIRECTORY);
    }
    Ok(())
}

/// A display name or a description: 1 to `max - 1` units.
pub fn check_text(text: &[u16], max: usize) -> Result<(), u32> {
    if text.is_empty() || text.len() >= max { Err(E_INVALIDARG) } else { Ok(()) }
}

/// The registry key (below `HKEY_CURRENT_USER`) of the SID-to-name mappings of the user.
pub const MAPPINGS_KEY: &str =
    "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppContainer\\Mappings";

/// The key of one container's mapping: `Mappings\<SID text>`.
pub fn mapping_key(container: &Sid) -> String {
    alloc::format!("{MAPPINGS_KEY}\\{}", sddl::to_string(container))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use alloc::vec::Vec;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn text(s: &Sid) -> String {
        sddl::to_string(s)
    }

    #[test]
    fn package_sids_are_the_sheets() {
        assert_eq!(text(&package_sid(&w("poly.test"))), "S-1-15-2-9258283-302506695-126040754-1514063749-3755156841-3572502495-4003830046");
        assert_eq!(package_sid(&w("POLY.TEST")), package_sid(&w("poly.test")));
        assert_eq!(text(&package_sid(&w("a"))), "S-1-15-2-3937069567-81109666-199193729-4036909440-616594969-3159470276-4124008600");
        assert!(is_package_sid(&package_sid(&w("x"))));
        assert!(!is_package_sid(&all_application_packages()));
    }

    #[test]
    fn capability_sids_are_the_sheets() {
        let (g, c) = capability_sids(&w("internetClient"));
        assert_eq!(text(&c), "S-1-15-3-1");
        assert_eq!(
            text(&g),
            "S-1-5-32-2779705173-1925339129-2667939958-2414465498-3395756507-4015878651-158944808-788332705"
        );
        assert_eq!(g.len(), 44);
        assert_eq!(c.len(), 16);
        assert_eq!(capability_sids(&w("INTERNETCLIENT")), (g, c));
        let (g, c) = capability_sids(&w("registryRead"));
        assert_eq!(
            text(&c),
            "S-1-15-3-1024-1065365936-1281604716-3511738428-1654721687-432734479-3232135806-4053264122-3456934681"
        );
        assert_eq!(c.len(), 48);
        assert_eq!(text(&g), "S-1-5-32-1065365936-1281604716-3511738428-1654721687-432734479-3232135806-4053264122-3456934681");
        // The empty name, an odd one, and one that is not well known although it looks so.
        assert_eq!(
            text(&capability_sids(&[]).1),
            "S-1-15-3-1024-1120186595-337443992-3371498394-616132505-3829509671-1284741988-463050148-1438143096"
        );
        assert_eq!(text(&capability_sids(&w("internetExplorer")).1).starts_with("S-1-15-3-1024-"), true);
        assert!(is_capability_sid(&capability_sids(&w("contacts")).1));
        assert!(!is_capability_sid(&all_application_packages()));
    }

    #[test]
    fn profile_names() {
        assert_eq!(check_profile_name(&[]), Err(E_INVALIDARG));
        assert_eq!(check_profile_name(&w(&"a".repeat(64))), Ok(()));
        assert_eq!(check_profile_name(&w(&"a".repeat(65))), Err(E_INVALIDARG));
        assert_eq!(check_profile_name(&w("a b")), Ok(()));
        assert_eq!(check_profile_name(&w("a\\b")), Err(E_PATH_NOT_FOUND));
        assert_eq!(check_profile_name(&w("a/b")), Err(E_PATH_NOT_FOUND));
        assert_eq!(check_profile_name(&w("a:b")), Err(E_DIRECTORY));
        assert_eq!(check_text(&w(&"d".repeat(511)), MAX_DISPLAY_NAME), Ok(()));
        assert_eq!(check_text(&w(&"d".repeat(512)), MAX_DISPLAY_NAME), Err(E_INVALIDARG));
        assert_eq!(check_text(&[], MAX_DESCRIPTION), Err(E_INVALIDARG));
        assert_eq!(check_text(&w(&"d".repeat(2047)), MAX_DESCRIPTION), Ok(()));
        assert_eq!(check_text(&w(&"d".repeat(2048)), MAX_DESCRIPTION), Err(E_INVALIDARG));
    }

    #[test]
    fn paths() {
        let s = package_sid(&w("poly.test"));
        assert_eq!(
            named_object_path(&s),
            "AppContainerNamedObjects\\S-1-15-2-9258283-302506695-126040754-1514063749-3755156841-3572502495-4003830046"
        );
        assert!(mapping_key(&s).ends_with("\\Mappings\\S-1-15-2-9258283-302506695-126040754-1514063749-3755156841-3572502495-4003830046"));
    }
}
