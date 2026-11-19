//! The checks of `advapi32!CreateProcessWithTokenW` (Learn, `winbase.h`; the sheet
//! `specs/windows/advapi32.dll/CreateProcessWithTokenW.yaml`), in the order the sheet observed:
//!
//! 1. The arguments ([`check_arguments`]): logon flags other than `LOGON_WITH_PROFILE` and
//!    `LOGON_NETCREDENTIALS_ONLY` are `ERROR_INVALID_PARAMETER`, as is `DETACHED_PROCESS` (the
//!    child always gets a console of its own or none, never "detached" from a caller it does not
//!    share one with) and a call that names no program (no application name and no or an empty
//!    command line); a NULL token is `ERROR_LOGON_FAILURE`.
//! 2. The caller's privilege: without `SeImpersonatePrivilege` every other call is
//!    `ERROR_PRIVILEGE_NOT_HELD`, whatever the token or the program.
//! 3. The token ([`check_token`]): what is not a token handle is `ERROR_INVALID_HANDLE` (the
//!    query of its type fails); a handle without every right the call needs
//!    ([`TOKEN_RIGHTS_NEEDED`]) is `ERROR_ACCESS_DENIED`; an impersonation token below
//!    `SecurityImpersonation` is `ERROR_BAD_IMPERSONATION_LEVEL`; the caller's own primary token
//!    (not a duplicate) is `ERROR_TOKEN_ALREADY_IN_USE`.
//! 4. The program, as `CreateProcessAsUserW` (missing file 2, missing path 3, not a program 193,
//!    a directory 5).
//!
//! What the child gets ([`child_directory`]; the environment is the token user's default,
//! `userenv!CreateEnvironmentBlock`): no handle of the caller (no inheritance at all), the token's
//! user's environment when none is given, and `System32` as its directory when none is given.

/// `LOGON_WITH_PROFILE`, `LOGON_NETCREDENTIALS_ONLY` (`winbase.h`).
pub const LOGON_WITH_PROFILE: u32 = 1;
pub const LOGON_NETCREDENTIALS_ONLY: u32 = 2;
/// `DETACHED_PROCESS` (`winbase.h`).
pub const DETACHED_PROCESS: u32 = 8;

/// `ERROR_ACCESS_DENIED`, `ERROR_INVALID_PARAMETER`, `ERROR_PRIVILEGE_NOT_HELD`,
/// `ERROR_LOGON_FAILURE`, `ERROR_BAD_IMPERSONATION_LEVEL`, `ERROR_TOKEN_ALREADY_IN_USE`
/// (`winerror.h`).
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_PRIVILEGE_NOT_HELD: u32 = 1314;
pub const ERROR_LOGON_FAILURE: u32 = 1326;
pub const ERROR_BAD_IMPERSONATION_LEVEL: u32 = 1346;
pub const ERROR_TOKEN_ALREADY_IN_USE: u32 = 1375;

/// `TOKEN_ASSIGN_PRIMARY`, `TOKEN_DUPLICATE`, `TOKEN_QUERY`, `TOKEN_ADJUST_DEFAULT`,
/// `TOKEN_ADJUST_SESSIONID` (`winnt.h`).
pub const TOKEN_ASSIGN_PRIMARY: u32 = 0x1;
pub const TOKEN_DUPLICATE: u32 = 0x2;
pub const TOKEN_QUERY: u32 = 0x8;
pub const TOKEN_ADJUST_DEFAULT: u32 = 0x80;
pub const TOKEN_ADJUST_SESSIONID: u32 = 0x100;

/// The rights the token handle needs. Learn names `TOKEN_QUERY`, `TOKEN_DUPLICATE` and
/// `TOKEN_ASSIGN_PRIMARY`, but the sheet saw a duplicate with exactly those refused (5) and only
/// one with all rights work: the call also sets the child token's session and default DACL
/// (`TOKEN_ADJUST_SESSIONID`, `TOKEN_ADJUST_DEFAULT`), which is Poly's reading of the rest.
pub const TOKEN_RIGHTS_NEEDED: u32 =
    TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ADJUST_DEFAULT | TOKEN_ADJUST_SESSIONID;

/// `SE_IMPERSONATE_PRIVILEGE` (`winnt.h`): the LUID's low part.
pub const SE_IMPERSONATE_PRIVILEGE: u32 = 29;

/// `TokenPrimary`, `TokenImpersonation` (`TOKEN_TYPE`), `SecurityImpersonation`
/// (`SECURITY_IMPERSONATION_LEVEL`), `winnt.h`.
pub const TOKEN_PRIMARY: u32 = 1;
pub const TOKEN_IMPERSONATION: u32 = 2;
pub const SECURITY_IMPERSONATION: u32 = 2;

/// Step 1: the arguments, before the privilege is looked at.
pub fn check_arguments(
    logon_flags: u32,
    creation_flags: u32,
    has_application: bool,
    command_line_len: Option<usize>,
    token_is_null: bool,
) -> Result<(), u32> {
    if logon_flags & !(LOGON_WITH_PROFILE | LOGON_NETCREDENTIALS_ONLY) != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if creation_flags & DETACHED_PROCESS != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if !has_application && command_line_len.is_none_or(|n| n == 0) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if token_is_null {
        return Err(ERROR_LOGON_FAILURE);
    }
    Ok(())
}

/// What the call learned about the token handle (step 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenFacts {
    /// The rights of the handle (`PUBLIC_OBJECT_BASIC_INFORMATION.GrantedAccess`).
    pub granted: u32,
    /// `TokenType`.
    pub token_type: u32,
    /// `TokenImpersonationLevel` (only read for an impersonation token).
    pub level: u32,
    /// Whether it is the caller's own primary token (the same `TokenId`).
    pub is_callers_primary: bool,
}

/// Step 3: whether the token may be used (the type query's failure is the caller's to report).
pub fn check_token(t: &TokenFacts) -> Result<(), u32> {
    if t.granted & TOKEN_RIGHTS_NEEDED != TOKEN_RIGHTS_NEEDED {
        return Err(ERROR_ACCESS_DENIED);
    }
    if t.token_type == TOKEN_IMPERSONATION && t.level < SECURITY_IMPERSONATION {
        return Err(ERROR_BAD_IMPERSONATION_LEVEL);
    }
    if t.is_callers_primary {
        return Err(ERROR_TOKEN_ALREADY_IN_USE);
    }
    Ok(())
}

/// Whether a `TOKEN_PRIVILEGES` (`winnt.h`: a count, then `LUID_AND_ATTRIBUTES` of 12 bytes)
/// holds `SeImpersonatePrivilege`.
pub fn holds_impersonate(privileges: &[u8]) -> bool {
    let Some(count) = privileges.get(..4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])) else {
        return false;
    };
    (0..count as usize).any(|i| {
        let at = 4 + i * 12;
        privileges.get(at..at + 8).is_some_and(|l| {
            u32::from_le_bytes([l[0], l[1], l[2], l[3]]) == SE_IMPERSONATE_PRIVILEGE
                && u32::from_le_bytes([l[4], l[5], l[6], l[7]]) == 0
        })
    })
}

/// The child's current directory: the one given, else the system directory (the sheet: not the
/// caller's).
pub fn child_directory(given: Option<&[u16]>, system_directory: &[u16]) -> alloc::vec::Vec<u16> {
    let mut d = given.unwrap_or(system_directory).to_vec();
    d.push(0);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_argument_errors_come_first_in_the_sheets_order() {
        assert_eq!(check_arguments(0, 0, true, Some(5), false), Ok(()));
        assert_eq!(check_arguments(3, 0x200, false, Some(5), false), Ok(()));
        for bad in [4, 0x80, 0xFFFF_FFFF] {
            assert_eq!(check_arguments(bad, 0, true, Some(5), false), Err(87), "{bad:#x}");
        }
        assert_eq!(check_arguments(0, DETACHED_PROCESS, true, None, false), Err(87));
        assert_eq!(check_arguments(0, 0, false, None, false), Err(87));
        assert_eq!(check_arguments(0, 0, false, Some(0), false), Err(87));
        assert_eq!(check_arguments(0, 0, true, None, true), Err(1326));
        // A bad flag is reported before a missing program and a NULL token.
        assert_eq!(check_arguments(0x80, 0, false, None, true), Err(87));
    }

    #[test]
    fn a_token_needs_every_right_a_level_and_must_not_be_the_callers() {
        let ok = TokenFacts { granted: 0xF01FF, token_type: TOKEN_PRIMARY, level: 0, is_callers_primary: false };
        assert_eq!(check_token(&ok), Ok(()));
        for granted in
            [TOKEN_QUERY, TOKEN_QUERY | TOKEN_DUPLICATE, TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY]
        {
            assert_eq!(check_token(&TokenFacts { granted, ..ok }), Err(5), "{granted:#x}");
        }
        let imp = TokenFacts { token_type: TOKEN_IMPERSONATION, level: SECURITY_IMPERSONATION, ..ok };
        assert_eq!(check_token(&imp), Ok(()));
        assert_eq!(check_token(&TokenFacts { level: 1, ..imp }), Err(1346));
        assert_eq!(check_token(&TokenFacts { is_callers_primary: true, ..ok }), Err(1375));
    }

    #[test]
    fn the_impersonate_privilege_is_found_by_its_luid() {
        let mut p = alloc::vec::Vec::new();
        p.extend_from_slice(&2u32.to_le_bytes());
        for (low, attrs) in [(23u32, 3u32), (29, 3)] {
            p.extend_from_slice(&low.to_le_bytes());
            p.extend_from_slice(&0u32.to_le_bytes());
            p.extend_from_slice(&attrs.to_le_bytes());
        }
        assert!(holds_impersonate(&p));
        p[4 + 12] = 30;
        assert!(!holds_impersonate(&p));
        assert!(!holds_impersonate(&[]));
    }

    #[test]
    fn the_child_starts_in_system32_unless_told() {
        let sys: alloc::vec::Vec<u16> = "C:\\Windows\\System32".encode_utf16().collect();
        let win: alloc::vec::Vec<u16> = "C:\\Windows".encode_utf16().collect();
        assert_eq!(child_directory(None, &sys), [&sys[..], &[0]].concat());
        assert_eq!(child_directory(Some(&win), &sys), [&win[..], &[0]].concat());
    }
}
