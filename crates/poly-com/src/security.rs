//! COM security across local processes (ADR 0220, B2c): proxy blankets, the arguments of
//! `CoInitializeSecurity`, and the security descriptors COM checks callers against.
//!
//! Rules from the sheets `CoSetProxyBlanket`, `CoQueryProxyBlanket`, `CoCopyProxy`,
//! `IClientSecurity`, `CoInitializeSecurity`, `IServerSecurity` and `AppID.Security`
//! (track `spec-win-com-sec`). Constants from `rpcdce.h`, `objidl.h`, `combaseapi.h` and
//! `winerror.h`; descriptor layouts from `winnt.h` (`SECURITY_DESCRIPTOR_RELATIVE`, `ACL`,
//! `ACE_HEADER`, `ACCESS_ALLOWED_ACE`).

use crate::hresult::{E_INVALIDARG, Hresult};
use alloc::string::String;
use alloc::vec::Vec;

/// `RPC_C_AUTHN_NONE`, `RPC_C_AUTHN_WINNT`, `RPC_C_AUTHN_DEFAULT` (`rpcdce.h`).
pub const RPC_C_AUTHN_NONE: u32 = 0;
pub const RPC_C_AUTHN_WINNT: u32 = 10;
pub const RPC_C_AUTHN_DEFAULT: u32 = 0xFFFF_FFFF;
/// `RPC_C_AUTHZ_NONE`, `RPC_C_AUTHZ_DEFAULT`.
pub const RPC_C_AUTHZ_NONE: u32 = 0;
pub const RPC_C_AUTHZ_DEFAULT: u32 = 0xFFFF_FFFF;
/// `RPC_C_AUTHN_LEVEL_NONE`, `RPC_C_AUTHN_LEVEL_PKT_PRIVACY`.
pub const RPC_C_AUTHN_LEVEL_NONE: u32 = 1;
pub const RPC_C_AUTHN_LEVEL_PKT_PRIVACY: u32 = 6;
/// `RPC_C_IMP_LEVEL_DEFAULT`, `_ANONYMOUS`, `_IDENTIFY`, `_IMPERSONATE`, `_DELEGATE`.
pub const RPC_C_IMP_LEVEL_DEFAULT: u32 = 0;
pub const RPC_C_IMP_LEVEL_ANONYMOUS: u32 = 1;
pub const RPC_C_IMP_LEVEL_IDENTIFY: u32 = 2;
pub const RPC_C_IMP_LEVEL_IMPERSONATE: u32 = 3;
pub const RPC_C_IMP_LEVEL_DELEGATE: u32 = 4;

/// `EOLE_AUTHENTICATION_CAPABILITIES` (`objidl.h`).
pub const EOAC_NONE: u32 = 0;
pub const EOAC_MUTUAL_AUTH: u32 = 0x1;
pub const EOAC_SECURE_REFS: u32 = 0x2;
pub const EOAC_ACCESS_CONTROL: u32 = 0x4;
pub const EOAC_APPID: u32 = 0x8;
pub const EOAC_DYNAMIC: u32 = 0x10;
pub const EOAC_STATIC_CLOAKING: u32 = 0x20;
pub const EOAC_DYNAMIC_CLOAKING: u32 = 0x40;
pub const EOAC_ANY_AUTHORITY: u32 = 0x80;
pub const EOAC_MAKE_FULLSIC: u32 = 0x100;
pub const EOAC_REQUIRE_FULLSIC: u32 = 0x200;
pub const EOAC_AUTO_IMPERSONATE: u32 = 0x400;
pub const EOAC_DEFAULT: u32 = 0x800;
pub const EOAC_DISABLE_AAA: u32 = 0x1000;
pub const EOAC_NO_CUSTOM_MARSHAL: u32 = 0x2000;
pub const EOAC_RESERVED1: u32 = 0x4000;

/// `COM_RIGHTS_EXECUTE`, `COM_RIGHTS_EXECUTE_LOCAL`, `COM_RIGHTS_ACTIVATE_LOCAL` (`objidl.h`).
pub const COM_RIGHTS_EXECUTE: u32 = 0x1;
pub const COM_RIGHTS_EXECUTE_LOCAL: u32 = 0x2;
pub const COM_RIGHTS_ACTIVATE_LOCAL: u32 = 0x8;

/// `E_ACCESSDENIED` (`winerror.h`).
pub const E_ACCESSDENIED: Hresult = 0x8007_0005_u32 as i32;
/// `RPC_E_CALL_COMPLETE`: no incoming call on this thread.
pub const RPC_E_CALL_COMPLETE: Hresult = 0x8001_0117_u32 as i32;
/// `HRESULT_FROM_WIN32(RPC_S_UNKNOWN_AUTHN_SERVICE)`, `(RPC_S_UNKNOWN_AUTHN_LEVEL)`.
pub const RPC_UNKNOWN_AUTHN_SERVICE: Hresult = 0x8007_06D3_u32 as i32;
pub const RPC_UNKNOWN_AUTHN_LEVEL: Hresult = 0x8007_06D4_u32 as i32;
/// `HRESULT_FROM_WIN32(ERROR_NONE_MAPPED)`: a principal name that names nobody.
pub const NONE_MAPPED: Hresult = 0x8007_0534_u32 as i32;
/// `HRESULT_FROM_WIN32(ERROR_INVALID_SECURITY_DESCR)`: a self-relative descriptor.
pub const INVALID_SECURITY_DESCR: Hresult = 0x8007_0551_u32 as i32;
/// `REGDB_E_INVALIDVALUE`: a `LaunchPermission` whose masks lack `COM_RIGHTS_EXECUTE`.
pub const REGDB_E_INVALIDVALUE: Hresult = 0x8004_0153_u32 as i32;
/// `RPC_S_NO_CALL_ACTIVE` (`winerror.h`), what `I_RpcOpenClientProcess` answers off a call.
pub const RPC_S_NO_CALL_ACTIVE: u32 = 1725;

/// The security settings of one interface proxy (sheet `CoQueryProxyBlanket`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blanket {
    pub authn: u32,
    pub authz: u32,
    /// `pServerPrincName`: the server's `<DOMAIN>\<user>` until a blanket with none is set.
    pub principal: Option<String>,
    pub level: u32,
    pub imp: u32,
    pub caps: u32,
    /// Set by `RPC_C_AUTHN_LEVEL_NONE` with `RPC_C_IMP_LEVEL_ANONYMOUS`: the next calls are refused
    /// with `E_ACCESSDENIED` without reaching the server (rule `effect_on_calls`).
    pub refuse: bool,
}

/// The capabilities a proxy blanket takes (rule `valid_settings_local`).
const PROXY_CAPS: [u32; 4] = [EOAC_NONE, EOAC_MUTUAL_AUTH, EOAC_STATIC_CLOAKING, EOAC_DYNAMIC_CLOAKING];

impl Blanket {
    /// A local proxy's blanket before anyone sets one (rule `local_proxy_defaults`).
    pub fn local_default(principal: Option<String>) -> Blanket {
        Blanket {
            authn: RPC_C_AUTHN_WINNT,
            authz: RPC_C_AUTHZ_NONE,
            principal,
            level: RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            imp: RPC_C_IMP_LEVEL_IDENTIFY,
            caps: EOAC_MUTUAL_AUTH,
            refuse: false,
        }
    }

    /// The blanket after `CoSetProxyBlanket` with these arguments, or why the call fails (the
    /// blanket stays as it was then). `principal` is the name given and whether it names an
    /// account; the local transport keeps NTLMSSP and `PKT_PRIVACY` whatever is asked (rules
    /// `valid_settings_local`, `blanket_readback_local`, `errors_leave_blanket_unchanged`).
    pub fn set(
        &self,
        authn: u32,
        authz: u32,
        principal: Option<(&str, bool)>,
        level: u32,
        imp: u32,
        caps: u32,
    ) -> Result<Blanket, Hresult> {
        if !matches!(authn, RPC_C_AUTHN_DEFAULT | RPC_C_AUTHN_WINNT | RPC_C_AUTHN_NONE) {
            return Err(RPC_UNKNOWN_AUTHN_SERVICE);
        }
        if level > RPC_C_AUTHN_LEVEL_PKT_PRIVACY {
            return Err(RPC_UNKNOWN_AUTHN_LEVEL);
        }
        if imp > RPC_C_IMP_LEVEL_DELEGATE || !PROXY_CAPS.contains(&caps) {
            return Err(E_INVALIDARG);
        }
        let principal = match principal {
            None => None,
            Some((name, true)) => Some(String::from(name)),
            Some((_, false)) => return Err(NONE_MAPPED),
        };
        let named = principal.is_some();
        let anonymous = level == RPC_C_AUTHN_LEVEL_NONE && imp == RPC_C_IMP_LEVEL_ANONYMOUS;
        let imp = match imp {
            RPC_C_IMP_LEVEL_DEFAULT => RPC_C_IMP_LEVEL_IDENTIFY,
            // Observed: NONE with ANONYMOUS reads back IMPERSONATE.
            RPC_C_IMP_LEVEL_ANONYMOUS if anonymous => RPC_C_IMP_LEVEL_IMPERSONATE,
            i => i,
        };
        Ok(Blanket {
            authn: RPC_C_AUTHN_WINNT,
            authz: if authz == RPC_C_AUTHZ_DEFAULT { RPC_C_AUTHZ_NONE } else { authz },
            principal,
            level: RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            imp,
            // Mutual authentication needs a principal name: without one it is not kept.
            caps: if caps == EOAC_MUTUAL_AUTH && !named { EOAC_NONE } else { caps },
            refuse: anonymous,
        })
    }

    /// The impersonation level a call through this proxy gives the server's thread token
    /// (`SECURITY_IMPERSONATION_LEVEL`: IDENTIFY -> 1, IMPERSONATE -> 2, DELEGATE -> 3).
    pub fn token_level(&self) -> u8 {
        token_level(self.imp)
    }
}

/// `SECURITY_IMPERSONATION_LEVEL` for an `RPC_C_IMP_LEVEL_*` (rule `impersonation_level_follows_call`).
pub fn token_level(imp: u32) -> u8 {
    match imp {
        RPC_C_IMP_LEVEL_IMPERSONATE => 2,
        RPC_C_IMP_LEVEL_DELEGATE => 3,
        RPC_C_IMP_LEVEL_ANONYMOUS => 0,
        _ => 1,
    }
}

/// The capabilities `CoInitializeSecurity` takes (rule `capabilities`).
const INIT_CAPS: u32 = EOAC_MUTUAL_AUTH
    | EOAC_SECURE_REFS
    | EOAC_APPID
    | EOAC_DYNAMIC
    | EOAC_STATIC_CLOAKING
    | EOAC_DYNAMIC_CLOAKING
    | EOAC_MAKE_FULLSIC
    | EOAC_REQUIRE_FULLSIC
    | EOAC_DISABLE_AAA
    | EOAC_NO_CUSTOM_MARSHAL
    | EOAC_RESERVED1;

/// The argument checks of `CoInitializeSecurity` that do not look at the descriptor (rules
/// `levels`, `capabilities`): levels 0..=6, impersonation 1..=4, the capabilities above with at
/// most one kind of cloaking. `E_INVALIDARG` otherwise.
pub fn check_init_args(level: u32, imp: u32, caps: u32) -> Result<(), Hresult> {
    if level > RPC_C_AUTHN_LEVEL_PKT_PRIVACY
        || !(RPC_C_IMP_LEVEL_ANONYMOUS..=RPC_C_IMP_LEVEL_DELEGATE).contains(&imp)
        || caps & !INIT_CAPS != 0
        || caps & (EOAC_STATIC_CLOAKING | EOAC_DYNAMIC_CLOAKING) == EOAC_STATIC_CLOAKING | EOAC_DYNAMIC_CLOAKING
    {
        return Err(E_INVALIDARG);
    }
    Ok(())
}

/// `SE_DACL_PRESENT`, `SE_SELF_RELATIVE` (`winnt.h`).
pub const SE_DACL_PRESENT: u16 = 0x0004;
pub const SE_SELF_RELATIVE: u16 = 0x8000;
/// `ACCESS_ALLOWED_ACE_TYPE`, `ACCESS_DENIED_ACE_TYPE`.
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;

/// One entry of a descriptor's DACL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ace {
    pub kind: u8,
    pub mask: u32,
}

/// What a self-relative security descriptor holds, as far as COM looks at it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub owner: bool,
    pub group: bool,
    /// `None`: no DACL present, or a NULL DACL (both admit everyone).
    pub dacl: Option<Vec<Ace>>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Read a `SECURITY_DESCRIPTOR_RELATIVE` (`winnt.h`): revision 1, the control word, and the
/// offsets of owner, group, SACL and DACL. `None` when it is not one.
pub fn parse_self_relative(b: &[u8]) -> Option<Descriptor> {
    if b.len() < 20 || b[0] != 1 {
        return None;
    }
    let control = u16_at(b, 2)?;
    if control & SE_SELF_RELATIVE == 0 {
        return None;
    }
    let owner = u32_at(b, 4)?;
    let group = u32_at(b, 8)?;
    let dacl_at = u32_at(b, 16)? as usize;
    let dacl = if control & SE_DACL_PRESENT == 0 || dacl_at == 0 {
        None
    } else {
        // ACL: revision, Sbz1, AclSize, AceCount, Sbz2; then the entries.
        let count = u16_at(b, dacl_at + 4)? as usize;
        let mut aces = Vec::with_capacity(count);
        let mut at = dacl_at + 8;
        for _ in 0..count {
            let kind = *b.get(at)?;
            let size = u16_at(b, at + 2)? as usize;
            let mask = u32_at(b, at + 4)?;
            if size < 8 {
                return None;
            }
            aces.push(Ace { kind, mask });
            at += size;
        }
        Some(aces)
    };
    Some(Descriptor { owner: owner != 0, group: group != 0, dacl })
}

impl Descriptor {
    /// Whether every entry grants or denies `COM_RIGHTS_EXECUTE` (rule `dacl_mask_validation`,
    /// `launch_permission_masks`).
    pub fn masks_valid(&self) -> bool {
        self.dacl.as_ref().is_none_or(|aces| {
            aces.iter()
                .filter(|a| matches!(a.kind, ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE))
                .all(|a| a.mask & COM_RIGHTS_EXECUTE != 0)
        })
    }

    /// The rights a `LaunchPermission` must grant to activate: `EXECUTE` alone for a legacy
    /// descriptor (entries with `EXECUTE` only), `EXECUTE | ACTIVATE_LOCAL` otherwise;
    /// `REGDB_E_INVALIDVALUE` when an entry lacks `EXECUTE` (sheet `AppID.Security`, rule
    /// `launch_permission_masks`: 0x1 admits, 0x3 refuses, 0xb admits, 0x2/0x8/0xa are invalid).
    pub fn launch_rights(&self) -> Result<u32, Hresult> {
        if !self.masks_valid() {
            return Err(REGDB_E_INVALIDVALUE);
        }
        let legacy = self.dacl.as_ref().is_none_or(|aces| aces.iter().all(|a| a.mask == COM_RIGHTS_EXECUTE));
        Ok(if legacy { COM_RIGHTS_EXECUTE } else { COM_RIGHTS_EXECUTE | COM_RIGHTS_ACTIVATE_LOCAL })
    }
}

/// `S-1-5-18` (LocalSystem) and `S-1-5-32-544` (Administrators), as `SID` bytes (`winnt.h`).
pub const SID_LOCAL_SYSTEM: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
pub const SID_ADMINISTRATORS: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 2, 0, 0];

/// A self-relative descriptor owned by `owner` (also its group) whose DACL allows
/// `COM_RIGHTS_EXECUTE` to each of `allowed` (an empty list: a DACL that admits nobody).
pub fn descriptor(owner: &[u8], allowed: &[&[u8]]) -> Vec<u8> {
    let mut acl = Vec::new();
    for sid in allowed {
        let size = (8 + sid.len()) as u16;
        acl.push(ACCESS_ALLOWED_ACE_TYPE);
        acl.push(0);
        acl.extend_from_slice(&size.to_le_bytes());
        acl.extend_from_slice(&COM_RIGHTS_EXECUTE.to_le_bytes());
        acl.extend_from_slice(sid);
    }
    let owner_at = 20u32;
    let group_at = owner_at + owner.len() as u32;
    let dacl_at = group_at + owner.len() as u32;
    let mut b = Vec::new();
    b.push(1);
    b.push(0);
    b.extend_from_slice(&(SE_SELF_RELATIVE | SE_DACL_PRESENT).to_le_bytes());
    b.extend_from_slice(&owner_at.to_le_bytes());
    b.extend_from_slice(&group_at.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&dacl_at.to_le_bytes());
    b.extend_from_slice(owner);
    b.extend_from_slice(owner);
    b.push(2); // ACL_REVISION
    b.push(0);
    b.extend_from_slice(&((8 + acl.len()) as u16).to_le_bytes());
    b.extend_from_slice(&(allowed.len() as u16).to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&acl);
    b
}

/// The access a process that did not call `CoInitializeSecurity` grants (sheets
/// `CoInitializeSecurity` rule `access_check`, `AppID.Security` rule `local_system_service_defaults`):
/// its own user, LocalSystem and Administrators. A same-user client is admitted whatever its token;
/// a LocalSystem service admits an elevated administrator and refuses a filtered one.
pub fn default_access(user: &[u8]) -> Vec<u8> {
    descriptor(user, &[user, &SID_LOCAL_SYSTEM, &SID_ADMINISTRATORS])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hresult::E_INVALIDARG;

    fn d() -> Blanket {
        Blanket::local_default(Some(String::from("PC\\server")))
    }

    #[test]
    fn blanket_readback_follows_the_local_transport() {
        let all = d().set(RPC_C_AUTHN_DEFAULT, RPC_C_AUTHZ_DEFAULT, None, 0, 0, 0).unwrap();
        assert_eq!((all.authn, all.authz, all.level, all.imp, all.caps), (10, 0, 6, 2, 0));
        assert_eq!(all.principal, None);
        let none = all.set(RPC_C_AUTHN_NONE, 0, None, RPC_C_AUTHN_LEVEL_NONE, 3, 0).unwrap();
        assert_eq!((none.authn, none.level, none.imp, none.refuse), (10, 6, 3, false));
        let z = none.set(RPC_C_AUTHN_DEFAULT, 77, None, 3, 3, 0).unwrap();
        assert_eq!(z.authz, 77);
        assert_eq!(z.set(RPC_C_AUTHN_DEFAULT, RPC_C_AUTHZ_DEFAULT, None, 3, 3, 1).unwrap().authz, 0);
        let anon = all.set(RPC_C_AUTHN_DEFAULT, RPC_C_AUTHZ_DEFAULT, None, 3, RPC_C_IMP_LEVEL_ANONYMOUS, 0).unwrap();
        assert_eq!((anon.imp, anon.refuse), (1, false));
        let refused = all.set(RPC_C_AUTHN_DEFAULT, RPC_C_AUTHZ_DEFAULT, None, 1, 1, 0).unwrap();
        assert_eq!((refused.imp, refused.refuse), (3, true));
        assert_eq!(all.set(10, 0, None, 3, 4, EOAC_DYNAMIC_CLOAKING).unwrap().caps, 0x40);
        assert_eq!(all.set(10, 0, None, 3, 3, EOAC_MUTUAL_AUTH).unwrap().caps, 0);
        assert_eq!(all.set(10, 0, None, 3, 3, EOAC_STATIC_CLOAKING).unwrap().caps, 0x20);
    }

    #[test]
    fn blanket_errors() {
        let b = d();
        assert_eq!(b.set(9, 0, None, 3, 3, 0), Err(RPC_UNKNOWN_AUTHN_SERVICE));
        assert_eq!(b.set(9999, 0, None, 3, 3, 0), Err(RPC_UNKNOWN_AUTHN_SERVICE));
        assert_eq!(b.set(10, 0, None, 7, 3, 0), Err(RPC_UNKNOWN_AUTHN_LEVEL));
        assert_eq!(b.set(10, 0, None, 3, 5, 0), Err(E_INVALIDARG));
        for caps in [0x10000, EOAC_ACCESS_CONTROL, EOAC_APPID, EOAC_SECURE_REFS] {
            assert_eq!(b.set(10, 0, None, 3, 3, caps), Err(E_INVALIDARG));
        }
        assert_eq!(b.set(10, 0, Some(("nobody\\nowhere", false)), 3, 3, 0), Err(NONE_MAPPED));
        assert_eq!(b.token_level(), 1);
        assert_eq!(token_level(RPC_C_IMP_LEVEL_DELEGATE), 3);
    }

    #[test]
    fn init_arguments() {
        for level in 0..=6 {
            assert_eq!(check_init_args(level, 3, 0), Ok(()));
        }
        for level in [7, 100, u32::MAX] {
            assert_eq!(check_init_args(level, 3, 0), Err(E_INVALIDARG));
        }
        for imp in [0, 5, u32::MAX] {
            assert_eq!(check_init_args(2, imp, 0), Err(E_INVALIDARG));
        }
        let ok = [0, 1, 2, 8, 0x10, 0x20, 0x40, 0x100, 0x200, 0x1000, 0x2000, 0x4000, 0x3000, 0x3];
        for caps in ok {
            assert_eq!(check_init_args(6, 3, caps), Ok(()), "{caps:#x}");
        }
        let bad = [0x4, 0x80, 0x400, 0x800, 0x60, 0xC, 0x8000, 0x10000, 0x8000_0000];
        for caps in bad {
            assert_eq!(check_init_args(6, 3, caps), Err(E_INVALIDARG), "{caps:#x}");
        }
    }

    fn with_masks(masks: &[u32]) -> Descriptor {
        Descriptor {
            owner: true,
            group: true,
            dacl: Some(masks.iter().map(|&mask| Ace { kind: ACCESS_ALLOWED_ACE_TYPE, mask }).collect()),
        }
    }

    #[test]
    fn launch_masks() {
        for m in [0x2, 0x8, 0xa] {
            assert_eq!(with_masks(&[m]).launch_rights(), Err(REGDB_E_INVALIDVALUE));
        }
        assert_eq!(with_masks(&[0x1]).launch_rights(), Ok(1));
        assert_eq!(with_masks(&[0x3]).launch_rights(), Ok(9));
        assert_eq!(with_masks(&[0xb]).launch_rights(), Ok(9));
        assert!(!with_masks(&[0x1, 0x2]).masks_valid());
        assert!(with_masks(&[0x1f, 0x1f_ffff]).masks_valid());
        assert!(Descriptor { owner: true, group: true, dacl: None }.masks_valid());
    }

    #[test]
    fn built_descriptors_parse() {
        let user = [1u8, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0xEA, 3, 0, 0];
        let sd = default_access(&user);
        let v = parse_self_relative(&sd).unwrap();
        assert!(v.owner && v.group);
        assert_eq!(v.dacl.as_ref().unwrap().len(), 3);
        assert!(v.dacl.unwrap().iter().all(|a| a.mask == COM_RIGHTS_EXECUTE && a.kind == 0));
        let none = parse_self_relative(&descriptor(&user, &[])).unwrap();
        assert_eq!(none.dacl, Some(Vec::new()));
        assert_eq!(parse_self_relative(&[0u8; 8]), None);
        let mut absolute = sd.clone();
        absolute[3] = 0;
        assert_eq!(parse_self_relative(&absolute), None);
    }
}
