//! The registry API's logic (ADR 0083): predefined keys, access masks, the
//! value-buffer rules of `RegQueryValueExW` / `RegGetValueW` /
//! `RegEnumValueW`, `RegGetValueW`'s type restrictions and string
//! termination, and the ANSI conversions of the `A` functions. The
//! functions themselves (`personalities/nt/win32/kernelbase/src/registry.rs`)
//! call our `ntdll`'s `Nt*Key` services (ADR 0066).
//!
//! Sources: Learn's pages of each function, "Predefined Keys",
//! "HKEY_CLASSES_ROOT Key", "Merged View of HKEY_CLASSES_ROOT", "Registry
//! Value Types", "Registry Key Security and Access Rights", and `winreg.h`
//! / `winnt.h` for the constants.
//!
//! # Predefined keys
//!
//! `winreg.h` defines them as `(HKEY)(ULONG_PTR)((LONG)0x80000000 + n)`,
//! sign-extended on x64; a zero-extended value is accepted too. Poly maps
//! them to NT paths ([`root_path`]): `HKEY_LOCAL_MACHINE` is
//! `\REGISTRY\MACHINE`, `HKEY_USERS` `\REGISTRY\USER`, `HKEY_CURRENT_USER`
//! `\REGISTRY\USER\<the token user's SID>`, `HKEY_CURRENT_CONFIG`
//! `\REGISTRY\MACHINE\SYSTEM\CurrentControlSet\Hardware Profiles\Current`,
//! `HKEY_CURRENT_USER_LOCAL_SETTINGS` `HKCU\Software\Classes\Local
//! Settings`. `HKEY_CLASSES_ROOT` is a view (Learn: "Merged View of
//! HKEY_CLASSES_ROOT"): a subkey is looked up in
//! `HKCU\Software\Classes` first, then in `HKLM\Software\Classes`; a new
//! subkey is created under `HKLM\Software\Classes` unless its parent exists
//! under the user's classes; the root itself (its values, its subkeys'
//! enumeration) is `HKLM\Software\Classes` — the enumeration is not merged
//! yet. The performance keys and `HKEY_DYN_DATA` are not served
//! (`ERROR_INVALID_HANDLE`, to be confirmed by the spec agent).

use crate::error::*;
use alloc::string::String;
use alloc::vec::Vec;

/// `ERROR_BADKEY` (`winerror.h`).
pub const ERROR_BADKEY: u32 = 1010;
/// `ERROR_DATATYPE_MISMATCH`, `ERROR_UNSUPPORTED_TYPE` (`winerror.h`).
pub const ERROR_DATATYPE_MISMATCH: u32 = 1629;
pub const ERROR_UNSUPPORTED_TYPE: u32 = 1630;

/// Value types (`winnt.h`).
pub const REG_NONE: u32 = 0;
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_BINARY: u32 = 3;
pub const REG_DWORD: u32 = 4;
pub const REG_MULTI_SZ: u32 = 7;
pub const REG_QWORD: u32 = 11;

/// Key rights (`winnt.h`).
pub const KEY_QUERY_VALUE: u32 = 0x0001;
pub const KEY_SET_VALUE: u32 = 0x0002;
pub const KEY_CREATE_SUB_KEY: u32 = 0x0004;
pub const KEY_ENUMERATE_SUB_KEYS: u32 = 0x0008;
pub const KEY_WOW64_64KEY: u32 = 0x0100;
pub const KEY_WOW64_32KEY: u32 = 0x0200;
pub const KEY_WOW64_RES: u32 = 0x0300;
pub const KEY_READ: u32 = 0x0002_0019;
pub const KEY_ALL_ACCESS: u32 = 0x000F_003F;
pub const DELETE: u32 = 0x0001_0000;
pub const MAXIMUM_ALLOWED: u32 = 0x0200_0000;

/// `RegCreateKeyExW` options (`winnt.h`).
pub const REG_OPTION_VOLATILE: u32 = 0x1;
pub const REG_OPTION_CREATE_LINK: u32 = 0x2;
pub const REG_OPTION_BACKUP_RESTORE: u32 = 0x4;
pub const REG_OPTION_OPEN_LINK: u32 = 0x8;
/// Dispositions.
pub const REG_CREATED_NEW_KEY: u32 = 1;
pub const REG_OPENED_EXISTING_KEY: u32 = 2;

/// `RegGetValueW` flags (`winreg.h`).
pub const RRF_RT_REG_NONE: u32 = 0x0000_0001;
pub const RRF_RT_REG_SZ: u32 = 0x0000_0002;
pub const RRF_RT_REG_EXPAND_SZ: u32 = 0x0000_0004;
pub const RRF_RT_REG_BINARY: u32 = 0x0000_0008;
pub const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
pub const RRF_RT_REG_MULTI_SZ: u32 = 0x0000_0020;
pub const RRF_RT_REG_QWORD: u32 = 0x0000_0040;
pub const RRF_RT_DWORD: u32 = RRF_RT_REG_BINARY | RRF_RT_REG_DWORD;
pub const RRF_RT_QWORD: u32 = RRF_RT_REG_BINARY | RRF_RT_REG_QWORD;
pub const RRF_RT_ANY: u32 = 0x0000_FFFF;
pub const RRF_SUBKEY_WOW6464KEY: u32 = 0x0001_0000;
pub const RRF_SUBKEY_WOW6432KEY: u32 = 0x0002_0000;
pub const RRF_NOEXPAND: u32 = 0x1000_0000;
pub const RRF_ZEROONFAILURE: u32 = 0x2000_0000;

/// The predefined keys (`winreg.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Predefined {
    ClassesRoot,
    CurrentUser,
    LocalMachine,
    Users,
    PerformanceData,
    CurrentConfig,
    DynData,
    CurrentUserLocalSettings,
    PerformanceText,
    PerformanceNlsText,
}

impl Predefined {
    /// Every predefined key with its `winreg.h` value (low 32 bits).
    pub const ALL: [(Predefined, u32); 10] = [
        (Predefined::ClassesRoot, 0x8000_0000),
        (Predefined::CurrentUser, 0x8000_0001),
        (Predefined::LocalMachine, 0x8000_0002),
        (Predefined::Users, 0x8000_0003),
        (Predefined::PerformanceData, 0x8000_0004),
        (Predefined::CurrentConfig, 0x8000_0005),
        (Predefined::DynData, 0x8000_0006),
        (Predefined::CurrentUserLocalSettings, 0x8000_0007),
        (Predefined::PerformanceText, 0x8000_0050),
        (Predefined::PerformanceNlsText, 0x8000_0060),
    ];

    /// Its index in [`Predefined::ALL`] (a slot for a cached handle).
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&(p, _)| p == self).expect("listed")
    }

    /// The `HKEY` value a program passes (sign-extended).
    pub fn hkey(self) -> u64 {
        i64::from(Self::ALL[self.index()].1 as i32) as u64
    }
}

/// The predefined key an `HKEY` names, if any.
pub fn predefined(hkey: u64) -> Option<Predefined> {
    let high = hkey >> 32;
    if high != 0 && high != 0xFFFF_FFFF {
        return None;
    }
    Predefined::ALL.iter().find(|&&(_, v)| v == hkey as u32).map(|&(p, _)| p)
}

/// `HKLM\Software\Classes`, the machine half of `HKEY_CLASSES_ROOT`.
pub const MACHINE_CLASSES: &str = "\\REGISTRY\\MACHINE\\SOFTWARE\\Classes";

/// The user's half of `HKEY_CLASSES_ROOT`.
pub fn user_classes(user_sid: &str) -> String {
    alloc::format!("\\REGISTRY\\USER\\{user_sid}\\Software\\Classes")
}

/// The NT path of a predefined key (for `HKEY_CLASSES_ROOT`, its machine
/// half); `None` if Poly does not serve it.
pub fn root_path(p: Predefined, user_sid: &str) -> Option<String> {
    Some(match p {
        Predefined::LocalMachine => String::from("\\REGISTRY\\MACHINE"),
        Predefined::Users => String::from("\\REGISTRY\\USER"),
        Predefined::CurrentUser => alloc::format!("\\REGISTRY\\USER\\{user_sid}"),
        Predefined::ClassesRoot => String::from(MACHINE_CLASSES),
        Predefined::CurrentConfig => {
            String::from("\\REGISTRY\\MACHINE\\SYSTEM\\CurrentControlSet\\Hardware Profiles\\Current")
        }
        Predefined::CurrentUserLocalSettings => alloc::format!("{}\\Local Settings", user_classes(user_sid)),
        Predefined::PerformanceData
        | Predefined::PerformanceText
        | Predefined::PerformanceNlsText
        | Predefined::DynData => {
            return None;
        }
    })
}

/// The access mask passed to `NtOpenKey`/`NtCreateKey`: the `KEY_WOW64_*`
/// view bits are the Win32 layer's (Poly has one view) and are dropped.
pub const fn key_access(sam: u32) -> u32 {
    sam & !KEY_WOW64_RES
}

/// The components of a subkey path, without empty ones (a leading,
/// trailing or doubled backslash is ignored; to be confirmed by the spec
/// agent).
pub fn components(path: &[u16]) -> Vec<&[u16]> {
    path.split(|&c| c == u16::from(b'\\')).filter(|c| !c.is_empty()).collect()
}

/// The subkey path without empty components, joined with backslashes.
pub fn normalized(path: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(path.len());
    for (i, c) in components(path).into_iter().enumerate() {
        if i > 0 {
            out.push(u16::from(b'\\'));
        }
        out.extend_from_slice(c);
    }
    out
}

/// What a value query does with the caller's buffer (`RegQueryValueExW`,
/// `RegEnumValueW`, `RegGetValueW` after its conversions): `Ok(true)` copy
/// the data, `Ok(false)` report the size only (no buffer), `Err` the error
/// (the size is reported in every case).
pub fn data_fit(len: usize, has_buffer: bool, capacity: u32) -> Result<bool, u32> {
    if !has_buffer {
        Ok(false)
    } else if (capacity as usize) < len {
        Err(ERROR_MORE_DATA)
    } else {
        Ok(true)
    }
}

/// Whether a value type holds strings.
pub const fn is_string(kind: u32) -> bool {
    matches!(kind, REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ)
}

fn units(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

fn bytes(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}

/// String data as the `A` functions return it: each UTF-16 unit to its
/// code page 1252 byte (a trailing odd byte is dropped). Other types are
/// unchanged.
pub fn to_ansi_data(kind: u32, data: &[u8]) -> Vec<u8> {
    if is_string(kind) { crate::text::to_ansi(&units(data)) } else { data.to_vec() }
}

/// String data the `A` functions are given, as UTF-16 for storage.
pub fn from_ansi_data(kind: u32, data: &[u8]) -> Vec<u8> {
    if is_string(kind) { bytes(&crate::text::from_ansi(data)) } else { data.to_vec() }
}

/// The `RRF_RT_*` bit of a value type (0 for types without one: only
/// `RRF_RT_ANY` admits them).
pub const fn type_bit(kind: u32) -> u32 {
    match kind {
        REG_NONE => RRF_RT_REG_NONE,
        REG_SZ => RRF_RT_REG_SZ,
        REG_EXPAND_SZ => RRF_RT_REG_EXPAND_SZ,
        REG_BINARY => RRF_RT_REG_BINARY,
        REG_DWORD => RRF_RT_REG_DWORD,
        REG_MULTI_SZ => RRF_RT_REG_MULTI_SZ,
        REG_QWORD => RRF_RT_REG_QWORD,
        _ => 0,
    }
}

/// `RegGetValueW`'s flag checks, before anything is opened. Learn: both
/// WOW64 flags are `ERROR_INVALID_PARAMETER`. Poly also refuses no type at
/// all and `RRF_RT_REG_EXPAND_SZ` without `RRF_NOEXPAND` (an expanded
/// string is a `REG_SZ`, so that restriction could never be met) — both to
/// be confirmed by the spec agent.
pub fn check_get_flags(flags: u32) -> Result<(), u32> {
    let both = RRF_SUBKEY_WOW6464KEY | RRF_SUBKEY_WOW6432KEY;
    if flags & both == both || flags & RRF_RT_ANY == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if flags & RRF_RT_ANY != RRF_RT_ANY && flags & RRF_RT_REG_EXPAND_SZ != 0 && flags & RRF_NOEXPAND == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(())
}

/// `RegGetValueW`'s result for a stored value: its type and data after the
/// type restriction, the expansion of a `REG_EXPAND_SZ` (unless
/// `RRF_NOEXPAND`; the type is then `REG_SZ`) and the string termination
/// Learn promises ("the function checks for a terminating null character.
/// If one is not found, the string is stored with a null terminator"; two
/// for `REG_MULTI_SZ`).
///
/// A type outside the restriction is `ERROR_UNSUPPORTED_TYPE`; a
/// `REG_BINARY` admitted only through `RRF_RT_DWORD` / `RRF_RT_QWORD` must
/// be 4 / 8 bytes, and a `REG_DWORD` / `REG_QWORD` must be its size, else
/// `ERROR_DATATYPE_MISMATCH` (to be confirmed by the spec agent).
pub fn get_value(
    flags: u32,
    kind: u32,
    data: &[u8],
    expand: &dyn Fn(&[u16]) -> Vec<u16>,
) -> Result<(u32, Vec<u8>), u32> {
    let restrict = flags & RRF_RT_ANY;
    let expanding = kind == REG_EXPAND_SZ && flags & RRF_NOEXPAND == 0;
    let effective = if expanding { REG_SZ } else { kind };
    if restrict != RRF_RT_ANY && restrict & type_bit(effective) == 0 {
        return Err(ERROR_UNSUPPORTED_TYPE);
    }
    match kind {
        REG_BINARY if restrict != RRF_RT_ANY && restrict & RRF_RT_REG_BINARY != 0 => {
            let sized = restrict & (RRF_RT_REG_DWORD | RRF_RT_REG_QWORD);
            if sized != 0
                && !(sized & RRF_RT_REG_DWORD != 0 && data.len() == 4)
                && !(sized & RRF_RT_REG_QWORD != 0 && data.len() == 8)
            {
                return Err(ERROR_DATATYPE_MISMATCH);
            }
        }
        REG_DWORD if data.len() != 4 => return Err(ERROR_DATATYPE_MISMATCH),
        REG_QWORD if data.len() != 8 => return Err(ERROR_DATATYPE_MISMATCH),
        _ => {}
    }
    if !is_string(kind) {
        return Ok((kind, data.to_vec()));
    }
    let mut s = units(data);
    if expanding {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        s = expand(&s[..end]);
    }
    let nuls = if kind == REG_MULTI_SZ { 2 } else { 1 };
    let have = s.iter().rev().take_while(|&&c| c == 0).count();
    // A REG_MULTI_SZ that is only one NUL (an empty list) gets a second.
    s.extend(core::iter::repeat_n(0, nuls - have.min(nuls)));
    Ok((effective, bytes(&s)))
}

/// The ANSI form of `RegGetValueW`'s result (`RegGetValueA`).
pub fn get_value_ansi(kind: u32, data: &[u8]) -> Vec<u8> {
    to_ansi_data(kind, data)
}

/// The fields of `KEY_FULL_INFORMATION` (`wdm.h`) that `RegQueryInfoKeyW`
/// reports: (subkeys, longest subkey name in characters, longest class,
/// values, longest value name in characters, longest data in bytes, last
/// write, class).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyInfo {
    pub subkeys: u32,
    pub max_subkey_len: u32,
    pub max_class_len: u32,
    pub values: u32,
    pub max_value_name_len: u32,
    pub max_value_len: u32,
    pub last_write: u64,
    pub class: Vec<u16>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn text_at(b: &[u8], at: usize, len: usize) -> Option<Vec<u16>> {
    Some(units(b.get(at..at + len)?))
}

/// Parse `KEY_FULL_INFORMATION`: `LastWriteTime`, `TitleIndex`,
/// `ClassOffset`, `ClassLength`, `SubKeys`, `MaxNameLen`, `MaxClassLen`,
/// `Values`, `MaxValueNameLen`, `MaxValueDataLen`, `Class` (lengths in
/// bytes).
pub fn parse_full_information(b: &[u8]) -> Option<KeyInfo> {
    let class_offset = u32_at(b, 12)? as usize;
    let class_len = u32_at(b, 16)? as usize;
    Some(KeyInfo {
        last_write: u64_at(b, 0)?,
        subkeys: u32_at(b, 20)?,
        max_subkey_len: u32_at(b, 24)? / 2,
        max_class_len: u32_at(b, 28)? / 2,
        values: u32_at(b, 32)?,
        max_value_name_len: u32_at(b, 36)? / 2,
        max_value_len: u32_at(b, 40)?,
        class: text_at(b, class_offset, class_len)?,
    })
}

/// Parse `KEY_NODE_INFORMATION`: (last write, name, class).
pub fn parse_node_information(b: &[u8]) -> Option<(u64, Vec<u16>, Vec<u16>)> {
    let class_offset = u32_at(b, 12)? as usize;
    let class_len = u32_at(b, 16)? as usize;
    let name_len = u32_at(b, 20)? as usize;
    Some((u64_at(b, 0)?, text_at(b, 24, name_len)?, text_at(b, class_offset, class_len)?))
}

/// Parse `KEY_VALUE_FULL_INFORMATION`: (type, name, data).
pub fn parse_value_full(b: &[u8]) -> Option<(u32, Vec<u16>, Vec<u8>)> {
    let kind = u32_at(b, 4)?;
    let data_offset = u32_at(b, 8)? as usize;
    let data_len = u32_at(b, 12)? as usize;
    let name_len = u32_at(b, 16)? as usize;
    Some((kind, text_at(b, 20, name_len)?, b.get(data_offset..data_offset + data_len)?.to_vec()))
}

/// Parse `KEY_VALUE_PARTIAL_INFORMATION`: (type, data).
pub fn parse_value_partial(b: &[u8]) -> Option<(u32, Vec<u8>)> {
    let kind = u32_at(b, 4)?;
    let len = u32_at(b, 8)? as usize;
    Some((kind, b.get(12..12 + len)?.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;

    #[test]
    fn predefined_keys() {
        assert_eq!(predefined(0xFFFF_FFFF_8000_0002), Some(Predefined::LocalMachine));
        assert_eq!(predefined(0x8000_0001), Some(Predefined::CurrentUser));
        assert_eq!(predefined(0x8000_0060), Some(Predefined::PerformanceNlsText));
        assert_eq!(predefined(0x1_8000_0000), None);
        assert_eq!(predefined(0x1234), None);
        assert_eq!(Predefined::ClassesRoot.hkey(), 0xFFFF_FFFF_8000_0000);
        let sid = "S-1-5-21-0-0-0-1000";
        assert_eq!(root_path(Predefined::CurrentUser, sid).as_deref(), Some("\\REGISTRY\\USER\\S-1-5-21-0-0-0-1000"));
        assert_eq!(root_path(Predefined::ClassesRoot, sid).as_deref(), Some(MACHINE_CLASSES));
        assert_eq!(user_classes(sid), "\\REGISTRY\\USER\\S-1-5-21-0-0-0-1000\\Software\\Classes");
        assert_eq!(root_path(Predefined::PerformanceData, sid), None);
        for (p, _) in Predefined::ALL {
            assert_eq!(predefined(p.hkey()), Some(p));
        }
    }

    #[test]
    fn paths_and_access() {
        let p = wide("\\Software\\\\Poly\\");
        assert_eq!(components(&p), [&wide("Software")[..], &wide("Poly")[..]]);
        assert_eq!(normalized(&p), wide("Software\\Poly"));
        assert!(components(&[]).is_empty());
        assert_eq!(key_access(KEY_READ | KEY_WOW64_64KEY), KEY_READ);
        assert_eq!(key_access(KEY_ALL_ACCESS | KEY_WOW64_32KEY), KEY_ALL_ACCESS);
    }

    #[test]
    fn buffer_rule() {
        assert_eq!(data_fit(8, false, 0), Ok(false));
        assert_eq!(data_fit(8, true, 8), Ok(true));
        assert_eq!(data_fit(8, true, 7), Err(ERROR_MORE_DATA));
    }

    fn sz(s: &str, nul: bool) -> Vec<u8> {
        let mut u = wide(s);
        if nul {
            u.push(0);
        }
        bytes(&u)
    }

    fn no_expand(s: &[u16]) -> Vec<u16> {
        s.to_vec()
    }

    #[test]
    fn get_value_restrictions() {
        let d = 7u32.to_le_bytes();
        assert_eq!(get_value(RRF_RT_REG_DWORD, REG_DWORD, &d, &no_expand), Ok((REG_DWORD, d.to_vec())));
        assert_eq!(get_value(RRF_RT_REG_SZ, REG_DWORD, &d, &no_expand), Err(ERROR_UNSUPPORTED_TYPE));
        assert_eq!(get_value(RRF_RT_DWORD, REG_BINARY, &d, &no_expand), Ok((REG_BINARY, d.to_vec())));
        assert_eq!(get_value(RRF_RT_DWORD, REG_BINARY, &[1, 2], &no_expand), Err(ERROR_DATATYPE_MISMATCH));
        assert_eq!(get_value(RRF_RT_QWORD, REG_BINARY, &[0; 8], &no_expand), Ok((REG_BINARY, [0; 8].to_vec())));
        assert_eq!(get_value(RRF_RT_REG_BINARY, REG_BINARY, &[1, 2], &no_expand), Ok((REG_BINARY, [1, 2].to_vec())));
        assert_eq!(get_value(RRF_RT_ANY, REG_BINARY, &[1, 2], &no_expand), Ok((REG_BINARY, [1, 2].to_vec())));
        assert_eq!(get_value(RRF_RT_ANY, REG_DWORD, &[1, 2], &no_expand), Err(ERROR_DATATYPE_MISMATCH));
        assert_eq!(get_value(RRF_RT_ANY, 99, &[5], &no_expand), Ok((99, [5].to_vec())));
        assert_eq!(get_value(RRF_RT_REG_NONE, 99, &[5], &no_expand), Err(ERROR_UNSUPPORTED_TYPE));
    }

    #[test]
    fn get_value_strings() {
        // A missing NUL is added; a present one is kept.
        assert_eq!(get_value(RRF_RT_REG_SZ, REG_SZ, &sz("ab", false), &no_expand), Ok((REG_SZ, sz("ab", true))));
        assert_eq!(get_value(RRF_RT_REG_SZ, REG_SZ, &sz("ab", true), &no_expand), Ok((REG_SZ, sz("ab", true))));
        // REG_MULTI_SZ ends with two.
        let mut m = sz("a", true);
        m.extend(sz("b", true));
        let mut m2 = m.clone();
        m2.extend([0, 0]);
        assert_eq!(get_value(RRF_RT_ANY, REG_MULTI_SZ, &m, &no_expand), Ok((REG_MULTI_SZ, m2.clone())));
        assert_eq!(get_value(RRF_RT_ANY, REG_MULTI_SZ, &m2, &no_expand), Ok((REG_MULTI_SZ, m2.clone())));
        assert_eq!(get_value(RRF_RT_ANY, REG_MULTI_SZ, &[], &no_expand), Ok((REG_MULTI_SZ, [0, 0, 0, 0].to_vec())));
        // REG_EXPAND_SZ is expanded to a REG_SZ unless RRF_NOEXPAND.
        let up = |s: &[u16]| -> Vec<u16> {
            s.iter().map(|&c| if (0x61..=0x7A).contains(&c) { c - 32 } else { c }).collect()
        };
        let e = sz("%x%", true);
        assert_eq!(get_value(RRF_RT_REG_SZ, REG_EXPAND_SZ, &e, &up), Ok((REG_SZ, sz("%X%", true))));
        assert_eq!(get_value(RRF_RT_ANY | RRF_NOEXPAND, REG_EXPAND_SZ, &e, &up), Ok((REG_EXPAND_SZ, e.clone())));
        assert_eq!(
            get_value(RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND, REG_EXPAND_SZ, &e, &up),
            Ok((REG_EXPAND_SZ, e.clone()))
        );
        assert_eq!(get_value(RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND, REG_SZ, &e, &up), Err(ERROR_UNSUPPORTED_TYPE));
    }

    #[test]
    fn get_flags() {
        assert_eq!(check_get_flags(RRF_RT_ANY), Ok(()));
        assert_eq!(
            check_get_flags(RRF_RT_ANY | RRF_SUBKEY_WOW6432KEY | RRF_SUBKEY_WOW6464KEY),
            Err(ERROR_INVALID_PARAMETER)
        );
        assert_eq!(check_get_flags(RRF_NOEXPAND), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(check_get_flags(RRF_RT_REG_EXPAND_SZ), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(check_get_flags(RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND), Ok(()));
    }

    #[test]
    fn ansi_data() {
        assert_eq!(to_ansi_data(REG_SZ, &sz("h\u{e9}", true)), [b'h', 0xE9, 0]);
        assert_eq!(to_ansi_data(REG_BINARY, &[1, 2, 3]), [1, 2, 3]);
        assert_eq!(from_ansi_data(REG_SZ, &[b'h', 0xE9, 0]), sz("h\u{e9}", true));
        assert_eq!(from_ansi_data(REG_DWORD, &[1, 0, 0, 0]), [1, 0, 0, 0]);
        assert_eq!(get_value_ansi(REG_MULTI_SZ, &sz("a", true)), [b'a', 0]);
    }

    #[test]
    fn parse_structures() {
        // KEY_FULL_INFORMATION with a two-character class.
        let mut f = Vec::new();
        f.extend(5u64.to_le_bytes());
        for v in [0u32, 44, 4, 3, 20, 6, 2, 10, 64] {
            f.extend(v.to_le_bytes());
        }
        f.extend(sz("cl", false));
        let info = parse_full_information(&f).unwrap();
        assert_eq!(
            info,
            KeyInfo {
                subkeys: 3,
                max_subkey_len: 10,
                max_class_len: 3,
                values: 2,
                max_value_name_len: 5,
                max_value_len: 64,
                last_write: 5,
                class: wide("cl")
            }
        );
        // KEY_NODE_INFORMATION "ab", class "c".
        let mut n = Vec::new();
        n.extend(9u64.to_le_bytes());
        for v in [0u32, 28, 2, 4] {
            n.extend(v.to_le_bytes());
        }
        n.extend(sz("ab", false));
        n.extend(sz("c", false));
        assert_eq!(parse_node_information(&n), Some((9, wide("ab"), wide("c"))));
        // KEY_VALUE_FULL_INFORMATION "v" = DWORD 1.
        let mut v = Vec::new();
        for x in [0u32, REG_DWORD, 22, 4, 2] {
            v.extend(x.to_le_bytes());
        }
        v.extend(sz("v", false));
        v.extend(1u32.to_le_bytes());
        assert_eq!(parse_value_full(&v), Some((REG_DWORD, wide("v"), 1u32.to_le_bytes().to_vec())));
        let mut p = Vec::new();
        for x in [0u32, REG_SZ, 2] {
            p.extend(x.to_le_bytes());
        }
        p.extend([b'x', 0]);
        assert_eq!(parse_value_partial(&p), Some((REG_SZ, [b'x', 0].to_vec())));
        assert_eq!(parse_value_partial(&p[..12]), None);
    }
}
