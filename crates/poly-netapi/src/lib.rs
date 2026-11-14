//! `secur32!GetUserNameExW` and `netapi32` for Poly's one user on a
//! stand-alone machine (ADR 0155, ADR 0083 for the user). Chrome asks
//! who the user is and whether the machine belongs to a domain (to
//! decide on enterprise policy); Poly answers as a workgroup computer
//! whose only account is `poly` (an administrator, RID 1002) and whose
//! name is the host name in upper case (NetBIOS style, at most 15
//! characters).
//!
//! Sources (clean room): Learn's pages "GetUserNameExW",
//! "EXTENDED_NAME_FORMAT", "NetUserGetInfo", "USER_INFO_0/1/10/20",
//! "NetApiBufferFree", "NetGetJoinInformation", "NETSETUP_JOIN_STATUS",
//! "DsRoleGetPrimaryDomainInformation",
//! "DSROLE_PRIMARY_DOMAIN_INFO_BASIC", "DsRoleFreeMemory",
//! "NetGetAadJoinInformation", "NetFreeAadJoinInformation"; the constants
//! and layouts of `secext.h`, `lmaccess.h`, `lmjoin.h`, `dsrole.h`,
//! `lmerr.h` and `winerror.h`. No Windows binary, Wine or ReactOS source
//! was looked at.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// The account's name (ADR 0066, `poly_security::POLY_USER_NAME`).
pub const USER_NAME: &str = "poly";
/// Its relative id: the last part of `poly_registry::defaults::POLY_USER_SID`.
pub const USER_RID: u32 = 1002;
/// The machine's three sub-authorities (`S-1-5-21-a-b-c`), the same as in that SID.
const MACHINE_SUBAUTHORITIES: [u32; 3] = [520_193_847, 884_726_105, 1_936_024_571];
/// The workgroup the machine is "joined" to.
pub const WORKGROUP: &str = "WORKGROUP";

/// `winerror.h`, `lmerr.h`.
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INVALID_LEVEL: u32 = 124;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_NONE_MAPPED: u32 = 1332;
pub const ERROR_BAD_NETPATH: u32 = 53;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const NERR_USER_NOT_FOUND: u32 = 2221;
/// `NERR_BadUsername` (`lmerr.h`): a NULL user name.
pub const NERR_BAD_USERNAME: u32 = 2202;

/// The computer name: the host name in upper case, cut to 15 characters.
pub fn computer_name(host: &str) -> String {
    let mut n: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(15).collect();
    n.make_ascii_uppercase();
    if n.is_empty() {
        n.push_str("POLY");
    }
    n
}

/// Whether `server` (a `servername` argument) means this computer: absent, empty, or the computer's name
/// with or without leading backslashes.
pub fn is_local(server: Option<&str>, host: &str) -> bool {
    match server {
        None => true,
        Some(s) => {
            let s = s.trim_start_matches('\\');
            s.is_empty() || s.eq_ignore_ascii_case(&computer_name(host)) || s == "."
        }
    }
}

/// `EXTENDED_NAME_FORMAT` (`secext.h`).
pub mod name_format {
    pub const NAME_UNKNOWN: u32 = 0;
    pub const NAME_FULLY_QUALIFIED_DN: u32 = 1;
    pub const NAME_SAM_COMPATIBLE: u32 = 2;
    pub const NAME_DISPLAY: u32 = 3;
    pub const NAME_UNIQUE_ID: u32 = 6;
    pub const NAME_CANONICAL: u32 = 7;
    pub const NAME_USER_PRINCIPAL: u32 = 8;
    pub const NAME_CANONICAL_EX: u32 = 9;
    pub const NAME_SERVICE_PRINCIPAL: u32 = 10;
    pub const NAME_DNS_DOMAIN: u32 = 12;
    pub const NAME_GIVEN_NAME: u32 = 13;
    pub const NAME_SURNAME: u32 = 14;
}

/// The user's name in `format` (`GetUserNameExW`): the SAM-compatible name is
/// `COMPUTER\poly`, the display name the account name; the formats that need a
/// directory (distinguished, canonical, user principal, ... names) are
/// `ERROR_NONE_MAPPED`, as for a local account on a computer that is not in a domain; a
/// format outside the enumeration is `ERROR_INVALID_PARAMETER`.
pub fn user_name_ex(format: u32, host: &str) -> Result<String, u32> {
    use name_format::*;
    match format {
        NAME_SAM_COMPATIBLE => Ok(alloc::format!("{}\\{USER_NAME}", computer_name(host))),
        NAME_DISPLAY => Ok(String::from(USER_NAME)),
        NAME_FULLY_QUALIFIED_DN
        | NAME_UNIQUE_ID
        | NAME_CANONICAL
        | NAME_USER_PRINCIPAL
        | NAME_CANONICAL_EX
        | NAME_SERVICE_PRINCIPAL
        | NAME_DNS_DOMAIN
        | NAME_GIVEN_NAME
        | NAME_SURNAME => Err(ERROR_NONE_MAPPED),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

/// `GetUserNameExW`'s buffer rule (Learn): `capacity` is `*nSize` on entry in characters. A short
/// buffer is `ERROR_MORE_DATA` with the size needed (the NUL included); on success the count is the
/// characters copied without the NUL. `Ok(count)` / `Err((error, size))`.
pub fn fit(name: &str, capacity: u32) -> Result<u32, (u32, u32)> {
    let units = name.encode_utf16().count() as u32;
    if capacity < units + 1 { Err((ERROR_MORE_DATA, units + 1)) } else { Ok(units) }
}

/// Little-endian output with absolute pointers (`base` is the address the bytes will have).
struct Out {
    buf: Vec<u8>,
    base: u64,
}

impl Out {
    fn new(base: u64) -> Out {
        Out { buf: Vec::new(), base }
    }
    fn alloc(&mut self, n: usize) -> usize {
        while !self.buf.len().is_multiple_of(8) {
            self.buf.push(0);
        }
        let at = self.buf.len();
        self.buf.resize(at + n, 0);
        at
    }
    fn ptr(&self, at: usize) -> u64 {
        self.base + at as u64
    }
    fn u32(&mut self, at: usize, v: u32) {
        self.buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, at: usize, v: u64) {
        self.buf[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    /// The user's SID, `S-1-5-21-a-b-c-RID` (revision 1, five sub-authorities, NT authority);
    /// the pointer to it.
    fn sid(&mut self) -> u64 {
        let at = self.alloc(8 + 5 * 4);
        self.buf[at] = 1;
        self.buf[at + 1] = 5;
        self.buf[at + 7] = 5;
        let subs = [21, MACHINE_SUBAUTHORITIES[0], MACHINE_SUBAUTHORITIES[1], MACHINE_SUBAUTHORITIES[2], USER_RID];
        for (i, v) in subs.iter().enumerate() {
            self.buf[at + 8 + i * 4..at + 12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        self.ptr(at)
    }
    /// A NUL-terminated UTF-16 string; the pointer to it.
    fn wide(&mut self, s: &str) -> u64 {
        let at = self.alloc((s.encode_utf16().count() + 1) * 2);
        for (i, c) in s.encode_utf16().enumerate() {
            self.buf[at + i * 2..at + i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        self.ptr(at)
    }
}

/// `USER_MAXSTORAGE_UNLIMITED` (`lmaccess.h`).
const USER_MAXSTORAGE_UNLIMITED: u32 = 0xFFFF_FFFF;

/// `USER_INFO_*` levels `NetUserGetInfo` serves.
pub mod user_level {
    pub const LEVEL_0: u32 = 0;
    pub const LEVEL_1: u32 = 1;
    pub const LEVEL_2: u32 = 2;
    pub const LEVEL_3: u32 = 3;
    pub const LEVEL_4: u32 = 4;
    pub const LEVEL_10: u32 = 10;
    pub const LEVEL_11: u32 = 11;
    pub const LEVEL_20: u32 = 20;
    pub const LEVEL_23: u32 = 23;
    pub const LEVEL_24: u32 = 24;
}

/// `UF_SCRIPT`, `UF_NORMAL_ACCOUNT` (`lmaccess.h`); `USER_PRIV_ADMIN`.
const UF_SCRIPT: u32 = 0x1;
const UF_NORMAL_ACCOUNT: u32 = 0x200;
const USER_PRIV_ADMIN: u32 = 2;

/// The `USER_INFO_<level>` structure for `user` as `NetUserGetInfo` returns it, for a buffer at
/// `base`. `server` and `host` decide whether the computer is this one (`ERROR_BAD_NETPATH`
/// otherwise); an unknown user is `NERR_UserNotFound`; a level Poly does not serve
/// `ERROR_INVALID_LEVEL`.
pub fn user_info(server: Option<&str>, user: &str, level: u32, host: &str, base: u64) -> Result<Vec<u8>, u32> {
    if !is_local(server, host) {
        return Err(ERROR_BAD_NETPATH);
    }
    if !user.eq_ignore_ascii_case(USER_NAME) {
        return Err(NERR_USER_NOT_FOUND);
    }
    let size = match level {
        user_level::LEVEL_0 => 8,
        user_level::LEVEL_1 => 56,
        user_level::LEVEL_2 => 152,
        user_level::LEVEL_3 => 184,
        user_level::LEVEL_4 => 192,
        user_level::LEVEL_10 => 32,
        user_level::LEVEL_11 => 128,
        user_level::LEVEL_20 => 32,
        user_level::LEVEL_23 => 40,
        user_level::LEVEL_24 => 32,
        _ => return Err(ERROR_INVALID_LEVEL),
    };
    let mut o = Out::new(base);
    let s = o.alloc(size);
    let name = o.wide(USER_NAME);
    let empty = o.wide("");
    // Every structure but level 24's starts with the pointer to the name.
    if level != user_level::LEVEL_24 {
        o.u64(s, name);
    }
    match level {
        user_level::LEVEL_1 => {
            // usri1_password stays NULL (Learn: not returned); password_age 0; priv admin;
            // home_dir, comment and script_path empty; UF_SCRIPT | UF_NORMAL_ACCOUNT.
            o.u32(s + 20, USER_PRIV_ADMIN);
            o.u64(s + 24, empty);
            o.u64(s + 32, empty);
            o.u32(s + 40, UF_SCRIPT | UF_NORMAL_ACCOUNT);
            o.u64(s + 48, empty);
        }
        user_level::LEVEL_2 | user_level::LEVEL_3 | user_level::LEVEL_4 => {
            // USER_INFO_2 / 3 / 4 (lmaccess.h) share their first 152 bytes: password NULL, an
            // administrator with UF_SCRIPT | UF_NORMAL_ACCOUNT, every string empty, no logon
            // limits (logon_hours NULL, units_per_week 0, max_storage USER_MAXSTORAGE_UNLIMITED).
            o.u32(s + 20, USER_PRIV_ADMIN);
            o.u64(s + 24, empty);
            o.u64(s + 32, empty);
            o.u32(s + 40, UF_SCRIPT | UF_NORMAL_ACCOUNT);
            o.u64(s + 48, empty);
            o.u64(s + 64, empty);
            o.u64(s + 72, empty);
            o.u64(s + 80, empty);
            o.u64(s + 88, empty);
            o.u32(s + 108, USER_MAXSTORAGE_UNLIMITED);
            o.u64(s + 136, empty);
            match level {
                user_level::LEVEL_2 => {}
                user_level::LEVEL_3 => {
                    // user_id, primary_group_id (DOMAIN_GROUP_RID_USERS 513), profile, home_dir_drive.
                    o.u32(s + 152, USER_RID);
                    o.u32(s + 156, 513);
                    o.u64(s + 160, empty);
                    o.u64(s + 168, empty);
                }
                _ => {
                    // user_sid, primary_group_id, profile, home_dir_drive.
                    let sid = o.sid();
                    o.u64(s + 152, sid);
                    o.u32(s + 160, 513);
                    o.u64(s + 168, empty);
                    o.u64(s + 176, empty);
                }
            }
        }
        user_level::LEVEL_11 => {
            // name, comment, usr_comment, full_name, priv, auth_flags, password_age, home_dir,
            // parms, logon_server, workstations (strings empty), max_storage.
            o.u64(s + 8, empty);
            o.u64(s + 16, empty);
            o.u64(s + 24, empty);
            o.u32(s + 32, USER_PRIV_ADMIN);
            o.u64(s + 48, empty);
            o.u64(s + 56, empty);
            o.u64(s + 80, empty);
            o.u64(s + 96, empty);
            o.u32(s + 104, USER_MAXSTORAGE_UNLIMITED);
        }
        user_level::LEVEL_23 => {
            // full_name, comment, flags, user_sid.
            o.u64(s + 8, empty);
            o.u64(s + 16, empty);
            o.u32(s + 24, UF_SCRIPT | UF_NORMAL_ACCOUNT);
            let sid = o.sid();
            o.u64(s + 32, sid);
        }
        user_level::LEVEL_24 => {
            // internet_identity FALSE, the two names empty, user_sid.
            o.u64(s + 8, empty);
            o.u64(s + 16, empty);
            let sid = o.sid();
            o.u64(s + 24, sid);
        }
        user_level::LEVEL_10 => {
            // comment, usr_comment, full_name.
            o.u64(s + 8, empty);
            o.u64(s + 16, empty);
            o.u64(s + 24, empty);
        }
        user_level::LEVEL_20 => {
            // full_name, comment, flags, user_id.
            o.u64(s + 8, empty);
            o.u64(s + 16, empty);
            o.u32(s + 24, UF_SCRIPT | UF_NORMAL_ACCOUNT);
            o.u32(s + 28, USER_RID);
        }
        _ => {}
    }
    Ok(o.buf)
}

/// `NETSETUP_JOIN_STATUS`: `NetSetupWorkgroupName` (`lmjoin.h`).
pub const NET_SETUP_WORKGROUP_NAME: u32 = 2;

/// `NetGetJoinInformation`: `(name, type)`, a workgroup. A remote server is `ERROR_BAD_NETPATH`.
pub fn join_information(server: Option<&str>, host: &str) -> Result<(&'static str, u32), u32> {
    if !is_local(server, host) {
        return Err(ERROR_BAD_NETPATH);
    }
    Ok((WORKGROUP, NET_SETUP_WORKGROUP_NAME))
}

/// `DSROLE_PRIMARY_DOMAIN_INFO_BASIC` (x64, 48 bytes): `MachineRole` 0, `Flags` 4, `DomainNameFlat`
/// 8, `DomainNameDns` 16, `DomainForestName` 24, `DomainGuid` 32. `DsRoleStandaloneWorkstation` (0),
/// no flags, the workgroup name flat and no DNS names.
pub fn dsrole_basic(server: Option<&str>, level: u32, host: &str, base: u64) -> Result<Vec<u8>, u32> {
    /// `DsRolePrimaryDomainInfoBasic`.
    const BASIC: u32 = 1;
    if level != BASIC {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if !is_local(server, host) {
        return Err(ERROR_BAD_NETPATH);
    }
    let mut o = Out::new(base);
    let s = o.alloc(48);
    let flat = o.wide(WORKGROUP);
    o.u64(s + 8, flat);
    Ok(o.buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }
    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }
    fn wide_at(b: &[u8], at: usize) -> String {
        let mut v = Vec::new();
        let mut i = at;
        while u16::from_le_bytes([b[i], b[i + 1]]) != 0 {
            v.push(u16::from_le_bytes([b[i], b[i + 1]]));
            i += 2;
        }
        String::from_utf16(&v).unwrap()
    }

    #[test]
    fn the_computer_name_is_netbios_style() {
        assert_eq!(computer_name("poly"), "POLY");
        assert_eq!(computer_name("my-very-long-host-name"), "MY-VERY-LONG-HO");
        assert_eq!(computer_name(""), "POLY");
        assert_eq!(computer_name("a b.c"), "ABC");
    }

    #[test]
    fn local_servers() {
        assert!(is_local(None, "poly"));
        assert!(is_local(Some(""), "poly"));
        assert!(is_local(Some("\\\\POLY"), "poly"));
        assert!(is_local(Some("poly"), "Poly"));
        assert!(!is_local(Some("\\\\OTHER"), "poly"));
    }

    #[test]
    fn user_names_by_format() {
        assert_eq!(user_name_ex(name_format::NAME_SAM_COMPATIBLE, "poly").as_deref(), Ok("POLY\\poly"));
        assert_eq!(user_name_ex(name_format::NAME_DISPLAY, "poly").as_deref(), Ok("poly"));
        assert_eq!(user_name_ex(name_format::NAME_USER_PRINCIPAL, "poly"), Err(ERROR_NONE_MAPPED));
        assert_eq!(user_name_ex(name_format::NAME_FULLY_QUALIFIED_DN, "poly"), Err(ERROR_NONE_MAPPED));
        assert_eq!(user_name_ex(name_format::NAME_UNKNOWN, "poly"), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(user_name_ex(99, "poly"), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn buffers_follow_learns_rule() {
        assert_eq!(fit("POLY\\poly", 10), Ok(9));
        assert_eq!(fit("POLY\\poly", 100), Ok(9));
        assert_eq!(fit("POLY\\poly", 9), Err((ERROR_MORE_DATA, 10)));
        assert_eq!(fit("poly", 0), Err((ERROR_MORE_DATA, 5)));
    }

    #[test]
    fn user_info_levels() {
        let b = user_info(None, "POLY", 1, "poly", 0x9000).unwrap();
        assert_eq!(wide_at(&b, (u64_at(&b, 0) - 0x9000) as usize), "poly");
        assert_eq!(u64_at(&b, 8), 0, "no password is returned");
        assert_eq!(u32_at(&b, 20), 2);
        assert_eq!(u32_at(&b, 40), 0x201);
        assert_eq!(wide_at(&b, (u64_at(&b, 24) - 0x9000) as usize), "");
        let b = user_info(Some("\\\\POLY"), "poly", 0, "poly", 0).unwrap();
        assert!(b.len() >= 8);
        let b = user_info(None, "poly", 20, "poly", 0x100).unwrap();
        assert_eq!(u32_at(&b, 28), 1002);
        assert_eq!(u32_at(&b, 24), 0x201);
        let b = user_info(None, "poly", 10, "poly", 0x100).unwrap();
        assert_ne!(u64_at(&b, 24), 0);
    }

    #[test]
    fn every_served_level_starts_with_the_name_and_pointers_stay_inside() {
        for level in [0, 1, 2, 3, 4, 10, 11, 20, 23] {
            let b = user_info(None, "poly", level, "poly", 0x4000).unwrap();
            assert_eq!(wide_at(&b, (u64_at(&b, 0) - 0x4000) as usize), "poly", "level {level}");
        }
        let sizes = [(2, 152), (3, 184), (4, 192), (11, 128), (23, 40), (24, 32)];
        for (level, size) in sizes {
            let b = user_info(None, "poly", level, "poly", 0x4000).unwrap();
            assert!(b.len() >= size, "level {level}");
        }
        // Level 24 has no name pointer; its user_sid (offset 24) points at S-1-5-21-a-b-c-RID.
        let b = user_info(None, "poly", 24, "poly", 0x4000).unwrap();
        let sid = (u64_at(&b, 24) - 0x4000) as usize;
        assert_eq!((b[sid], b[sid + 1]), (1, 5));
        assert_eq!(u32_at(&b, sid + 8 + 16), USER_RID);
        // Level 3's user_id and level 4's SID agree with the RID.
        let b = user_info(None, "poly", 3, "poly", 0).unwrap();
        assert_eq!(u32_at(&b, 152), USER_RID);
        let b = user_info(None, "poly", 4, "poly", 0x4000).unwrap();
        let sid = (u64_at(&b, 152) - 0x4000) as usize;
        assert_eq!(u32_at(&b, sid + 8 + 16), USER_RID);
        let b = user_info(None, "poly", 11, "poly", 0x4000).unwrap();
        assert_eq!(u32_at(&b, 32), 2);
    }

    #[test]
    fn user_info_errors() {
        assert_eq!(user_info(None, "nobody", 1, "poly", 0), Err(NERR_USER_NOT_FOUND));
        // The sheet: these levels are not served; the user is looked up before the level is judged.
        for level in [5, 12, 21, 22, 25, 1003, 0xFFFF_FFFF] {
            assert_eq!(user_info(None, "poly", level, "poly", 0), Err(ERROR_INVALID_LEVEL), "{level}");
            assert_eq!(user_info(None, "nobody", level, "poly", 0), Err(NERR_USER_NOT_FOUND), "{level}");
        }
        assert_eq!(user_info(Some("\\\\FAR"), "poly", 1, "poly", 0), Err(ERROR_BAD_NETPATH));
    }

    #[test]
    fn workgroup_answers() {
        assert_eq!(join_information(None, "poly"), Ok(("WORKGROUP", 2)));
        assert_eq!(join_information(Some("\\\\FAR"), "poly"), Err(ERROR_BAD_NETPATH));
        let b = dsrole_basic(None, 1, "poly", 0x500).unwrap();
        assert_eq!(u32_at(&b, 0), 0, "stand-alone workstation");
        assert_eq!(u32_at(&b, 4), 0);
        assert_eq!(wide_at(&b, (u64_at(&b, 8) - 0x500) as usize), "WORKGROUP");
        assert_eq!(u64_at(&b, 16), 0);
        assert_eq!(dsrole_basic(None, 2, "poly", 0), Err(ERROR_INVALID_PARAMETER));
    }
}
