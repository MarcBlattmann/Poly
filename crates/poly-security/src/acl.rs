//! `ACL` and its ACEs (`winnt.h`), on the caller's bytes.
//!
//! An ACL is an 8-byte header — `AclRevision` (byte), `Sbz1`, `AclSize`
//! (word, the whole buffer), `AceCount` (word), `Sbz2` — followed by
//! `AceCount` ACEs packed one after the other. Every ACE starts with an
//! `ACE_HEADER` — `AceType`, `AceFlags` (bytes), `AceSize` (word) — and the
//! common ones (`ACCESS_ALLOWED_ACE`, `ACCESS_DENIED_ACE`,
//! `SYSTEM_AUDIT_ACE`, `SYSTEM_MANDATORY_LABEL_ACE`) continue with an
//! `ACCESS_MASK` and the SID (`SidStart`).
//!
//! The functions take the ACL's bytes (`AclSize` of them, which the DLL
//! reads from the header first) and return the Win32 error Learn names.
//!
//! Choices to be confirmed by the spec agent:
//! - `InitializeAcl` with a length below 8 is `ERROR_INSUFFICIENT_BUFFER`,
//!   above 0xFFFF `ERROR_INVALID_PARAMETER`; a length that is not a multiple
//!   of 4 is rounded down (Learn: "this value must be DWORD-aligned").
//! - Adding an ACE whose revision is newer than the ACL's raises the ACL's
//!   revision; an older one leaves it. A revision outside 2..=4 is
//!   `ERROR_REVISION_MISMATCH`.
//! - Validity (`IsValidAcl`): revision 2..=4, size at least 8 and a
//!   multiple of 4, and every ACE inside the size, at least 4 bytes, a
//!   multiple of 4, with a valid SID where its type has one.

use crate::error::*;
use crate::sid::{self, Sid};
use alloc::vec::Vec;

/// `ACL_REVISION`, `ACL_REVISION_DS` (and the range, `MIN_ACL_REVISION` ..
/// `MAX_ACL_REVISION`).
pub const ACL_REVISION: u8 = 2;
pub const ACL_REVISION_DS: u8 = 4;
pub const MIN_ACL_REVISION: u8 = 2;
pub const MAX_ACL_REVISION: u8 = 4;
/// `sizeof(ACL)`.
pub const ACL_HEADER: usize = 8;
/// `sizeof(ACE_HEADER)`.
pub const ACE_HEADER: usize = 4;
/// `sizeof(ACCESS_ALLOWED_ACE) - sizeof(DWORD)`: header and mask, before
/// the SID.
pub const ACE_FIXED: usize = 8;

/// ACE types (`winnt.h`).
pub const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
pub const ACCESS_DENIED_ACE_TYPE: u8 = 1;
pub const SYSTEM_AUDIT_ACE_TYPE: u8 = 2;
pub const SYSTEM_ALARM_ACE_TYPE: u8 = 3;
pub const SYSTEM_MANDATORY_LABEL_ACE_TYPE: u8 = 0x11;

/// ACE flags (`winnt.h`).
pub const OBJECT_INHERIT_ACE: u8 = 0x1;
pub const CONTAINER_INHERIT_ACE: u8 = 0x2;
pub const NO_PROPAGATE_INHERIT_ACE: u8 = 0x4;
pub const INHERIT_ONLY_ACE: u8 = 0x8;
pub const INHERITED_ACE: u8 = 0x10;
pub const VALID_INHERIT_FLAGS: u8 = 0x1F;
pub const SUCCESSFUL_ACCESS_ACE_FLAG: u8 = 0x40;
pub const FAILED_ACCESS_ACE_FLAG: u8 = 0x80;

/// `ACL_INFORMATION_CLASS` (`winnt.h`).
pub const ACL_REVISION_INFORMATION: u32 = 1;
pub const ACL_SIZE_INFORMATION: u32 = 2;

/// `AddAce`'s "append" index (`MAXDWORD`).
pub const APPEND: u32 = u32::MAX;

fn word(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn set_word(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

/// The header: (revision, size, ACE count). `None` if `b` is shorter.
pub fn header(b: &[u8]) -> Option<(u8, u16, u16)> {
    (b.len() >= ACL_HEADER).then(|| (b[0], word(b, 2), word(b, 4)))
}

/// Whether ACEs of this type carry a mask and a SID right after the header.
const fn has_sid(kind: u8) -> bool {
    matches!(
        kind,
        ACCESS_ALLOWED_ACE_TYPE
            | ACCESS_DENIED_ACE_TYPE
            | SYSTEM_AUDIT_ACE_TYPE
            | SYSTEM_ALARM_ACE_TYPE
            | SYSTEM_MANDATORY_LABEL_ACE_TYPE
    )
}

/// `InitializeAcl` on a buffer of `length` bytes (`buf` holds them).
pub fn initialize(buf: &mut [u8], length: u32, revision: u32) -> Result<(), u32> {
    if !(u32::from(MIN_ACL_REVISION)..=u32::from(MAX_ACL_REVISION)).contains(&revision) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if (length as usize) < ACL_HEADER {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    if length > 0xFFFF {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let size = (length & !3) as u16;
    buf[..ACL_HEADER].copy_from_slice(&[revision as u8, 0, 0, 0, 0, 0, 0, 0]);
    set_word(buf, 2, size);
    Ok(())
}

/// The offsets of the ACEs (start, size), checking the structure.
fn walk(b: &[u8]) -> Result<Vec<(usize, usize)>, u32> {
    let (rev, size, count) = header(b).ok_or(ERROR_INVALID_ACL)?;
    let size = usize::from(size);
    if !(MIN_ACL_REVISION..=MAX_ACL_REVISION).contains(&rev) || size < ACL_HEADER || size % 4 != 0 || b.len() < size {
        return Err(ERROR_INVALID_ACL);
    }
    let mut at = ACL_HEADER;
    let mut aces = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        if at + ACE_HEADER > size {
            return Err(ERROR_INVALID_ACL);
        }
        let ace_size = usize::from(word(b, at + 2));
        if ace_size < ACE_HEADER || ace_size % 4 != 0 || at + ace_size > size {
            return Err(ERROR_INVALID_ACL);
        }
        if has_sid(b[at]) {
            let sid = &b[(at + ACE_FIXED).min(at + ace_size)..at + ace_size];
            if ace_size < ACE_FIXED || sid::valid_length(sid).is_none() {
                return Err(ERROR_INVALID_ACL);
            }
        }
        aces.push((at, ace_size));
        at += ace_size;
    }
    Ok(aces)
}

/// `IsValidAcl`.
pub fn is_valid(b: &[u8]) -> bool {
    walk(b).is_ok()
}

/// The bytes in use: the header and every ACE.
pub fn bytes_in_use(b: &[u8]) -> Result<usize, u32> {
    Ok(walk(b)?.last().map_or(ACL_HEADER, |&(at, len)| at + len))
}

/// `GetAce`: the offset of ACE `index`.
pub fn ace_offset(b: &[u8], index: u32) -> Result<usize, u32> {
    walk(b)?.get(index as usize).map(|&(at, _)| at).ok_or(ERROR_INVALID_PARAMETER)
}

/// `FindFirstFreeAce`: the offset just past the last ACE.
pub fn first_free(b: &[u8]) -> Result<usize, u32> {
    bytes_in_use(b)
}

fn check_revision(b: &mut [u8], revision: u32) -> Result<(), u32> {
    if !(u32::from(MIN_ACL_REVISION)..=u32::from(MAX_ACL_REVISION)).contains(&revision) {
        return Err(ERROR_REVISION_MISMATCH);
    }
    if revision as u8 > b[0] {
        b[0] = revision as u8;
    }
    Ok(())
}

/// One ACE with a mask and a SID.
pub fn ace(kind: u8, flags: u8, mask: u32, sid: &[u8]) -> Result<Vec<u8>, u32> {
    let len = sid::valid_length(sid).ok_or(ERROR_INVALID_SID)?;
    let size = ACE_FIXED + len;
    let mut v = Vec::with_capacity(size);
    v.extend_from_slice(&[kind, flags]);
    v.extend_from_slice(&(size as u16).to_le_bytes());
    v.extend_from_slice(&mask.to_le_bytes());
    v.extend_from_slice(&sid[..len]);
    Ok(v)
}

/// `AddAccessAllowedAce(Ex)`, `AddAccessDeniedAce(Ex)`,
/// `AddAuditAccessAce(Ex)`, `AddMandatoryAce`: append one ACE. `flags`
/// must be inheritance flags (plus the two audit flags for an audit ACE).
pub fn add(b: &mut [u8], revision: u32, kind: u8, flags: u32, mask: u32, sid: &[u8]) -> Result<(), u32> {
    let allowed = if kind == SYSTEM_AUDIT_ACE_TYPE {
        u32::from(VALID_INHERIT_FLAGS | SUCCESSFUL_ACCESS_ACE_FLAG | FAILED_ACCESS_ACE_FLAG)
    } else {
        u32::from(VALID_INHERIT_FLAGS)
    };
    if flags & !allowed != 0 {
        return Err(ERROR_INVALID_FLAGS);
    }
    let new = ace(kind, flags as u8, mask, sid)?;
    insert(b, revision, APPEND, &new)
}

/// `AddAce`: insert the ACEs in `list` before ACE `start` (or at the end
/// for [`APPEND`] or any index past the last).
pub fn insert(b: &mut [u8], revision: u32, start: u32, list: &[u8]) -> Result<(), u32> {
    let aces = walk(b)?;
    // The list must be whole ACEs.
    let mut n = 0u16;
    let mut at = 0;
    while at < list.len() {
        if at + ACE_HEADER > list.len() {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let s = usize::from(word(list, at + 2));
        if s < ACE_HEADER || at + s > list.len() {
            return Err(ERROR_INVALID_PARAMETER);
        }
        at += s;
        n += 1;
    }
    let used = aces.last().map_or(ACL_HEADER, |&(a, l)| a + l);
    let size = usize::from(word(b, 2));
    if used + list.len() > size {
        return Err(ERROR_ALLOTTED_SPACE_EXCEEDED);
    }
    check_revision(b, revision)?;
    let pos = aces.get(start as usize).map_or(used, |&(a, _)| a);
    b.copy_within(pos..used, pos + list.len());
    b[pos..pos + list.len()].copy_from_slice(list);
    let count = word(b, 4) + n;
    set_word(b, 4, count);
    Ok(())
}

/// `DeleteAce`.
pub fn delete(b: &mut [u8], index: u32) -> Result<(), u32> {
    let aces = walk(b)?;
    let &(at, len) = aces.get(index as usize).ok_or(ERROR_INVALID_PARAMETER)?;
    let used = aces.last().map_or(ACL_HEADER, |&(a, l)| a + l);
    b.copy_within(at + len..used, at);
    b[used - len..used].fill(0);
    let count = word(b, 4) - 1;
    set_word(b, 4, count);
    Ok(())
}

/// `GetAclInformation`: `ACL_REVISION_INFORMATION` (`AclRevision`) or
/// `ACL_SIZE_INFORMATION` (`AceCount`, `AclBytesInUse`, `AclBytesFree`), as
/// bytes; the caller checks its buffer is at least that long
/// (`ERROR_INSUFFICIENT_BUFFER`).
pub fn information(b: &[u8], class: u32) -> Result<Vec<u8>, u32> {
    let used = bytes_in_use(b)?;
    let (rev, size, count) = header(b).ok_or(ERROR_INVALID_ACL)?;
    let mut v = Vec::new();
    match class {
        ACL_REVISION_INFORMATION => v.extend_from_slice(&u32::from(rev).to_le_bytes()),
        ACL_SIZE_INFORMATION => {
            v.extend_from_slice(&u32::from(count).to_le_bytes());
            v.extend_from_slice(&(used as u32).to_le_bytes());
            v.extend_from_slice(&(usize::from(size) - used).to_le_bytes()[..4]);
        }
        _ => return Err(ERROR_INVALID_PARAMETER),
    }
    Ok(v)
}

/// A whole ACL with exactly room for `aces` (type, flags, mask, SID).
pub fn build(revision: u8, aces: &[(u8, u8, u32, &Sid)]) -> Vec<u8> {
    let mut body = Vec::new();
    for &(kind, flags, mask, s) in aces {
        body.extend(ace(kind, flags, mask, &s.to_bytes()).expect("a valid SID"));
    }
    let size = ACL_HEADER + body.len();
    let mut v = Vec::with_capacity(size);
    v.extend_from_slice(&[revision, 0]);
    v.extend_from_slice(&(size as u16).to_le_bytes());
    v.extend_from_slice(&(aces.len() as u16).to_le_bytes());
    v.extend_from_slice(&[0, 0]);
    v.extend(body);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sid::{builtin, local_system, world};

    fn acl(len: usize) -> Vec<u8> {
        let mut b = alloc::vec![0xAAu8; len];
        initialize(&mut b, len as u32, u32::from(ACL_REVISION)).unwrap();
        b
    }

    #[test]
    fn initialize_rules() {
        let mut b = [0u8; 64];
        assert_eq!(initialize(&mut b, 64, 1), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(initialize(&mut b, 4, 2), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(initialize(&mut b, 0x1_0000, 2), Err(ERROR_INVALID_PARAMETER));
        initialize(&mut b, 62, 4).unwrap();
        assert_eq!(header(&b), Some((4, 60, 0)));
        assert!(is_valid(&b));
        assert_eq!(information(&b, ACL_SIZE_INFORMATION).unwrap(), [0, 0, 0, 0, 8, 0, 0, 0, 52, 0, 0, 0]);
        assert_eq!(information(&b, ACL_REVISION_INFORMATION).unwrap(), [4, 0, 0, 0]);
        assert_eq!(information(&b, 3), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn add_get_insert_delete() {
        let everyone = world().to_bytes();
        let admins = builtin(544).to_bytes();
        // Learn's size rule: header + (ACE - SidStart + SID) per ACE.
        let mut b = acl(8 + (8 + 12) + (8 + 16));
        add(&mut b, 2, ACCESS_ALLOWED_ACE_TYPE, 0, 0x1F_01FF, &everyone).unwrap();
        add(&mut b, 2, ACCESS_DENIED_ACE_TYPE, u32::from(CONTAINER_INHERIT_ACE), 0x10000, &admins).unwrap();
        assert_eq!(header(&b), Some((2, 52, 2)));
        assert_eq!(add(&mut b, 2, 0, 0, 1, &everyone), Err(ERROR_ALLOTTED_SPACE_EXCEEDED));
        assert_eq!(ace_offset(&b, 0), Ok(8));
        assert_eq!(ace_offset(&b, 1), Ok(28));
        assert_eq!(ace_offset(&b, 2), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(&b[28..32], &[1, 2, 24, 0]);
        assert_eq!(&b[36..52], &admins[..]);
        assert_eq!(first_free(&b), Ok(52));
        // Delete the first and the second moves down.
        delete(&mut b, 0).unwrap();
        assert_eq!(header(&b), Some((2, 52, 1)));
        assert_eq!(&b[8..12], &[1, 2, 24, 0]);
        // Insert it back at the front with AddAce.
        let e = ace(ACCESS_ALLOWED_ACE_TYPE, 0, 5, &everyone).unwrap();
        insert(&mut b, 2, 0, &e).unwrap();
        assert_eq!(header(&b), Some((2, 52, 2)));
        assert_eq!(&b[8..20], &e[..12]);
        assert!(is_valid(&b));
    }

    #[test]
    fn argument_errors() {
        let mut b = acl(64);
        let s = world().to_bytes();
        assert_eq!(add(&mut b, 2, 0, 0x20, 1, &s), Err(ERROR_INVALID_FLAGS));
        assert_eq!(add(&mut b, 2, SYSTEM_AUDIT_ACE_TYPE, 0xC0, 1, &s), Ok(()));
        assert_eq!(add(&mut b, 9, 0, 0, 1, &s), Err(ERROR_REVISION_MISMATCH));
        assert_eq!(add(&mut b, 2, 0, 0, 1, &[2, 0, 0, 0, 0, 0, 0, 0]), Err(ERROR_INVALID_SID));
        // A newer revision raises the ACL's.
        add(&mut b, 4, 0, 0, 1, &s).unwrap();
        assert_eq!(b[0], 4);
        add(&mut b, 2, 0, 0, 1, &s).unwrap_err();
        let mut bad = acl(16);
        bad[0] = 7;
        assert!(!is_valid(&bad));
        assert_eq!(add(&mut bad, 2, 0, 0, 1, &s), Err(ERROR_INVALID_ACL));
        let mut broken = acl(16);
        broken[4] = 1; // one ACE claimed, none there
        assert!(!is_valid(&broken));
        assert_eq!(delete(&mut acl(16), 0), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn built_acls_are_valid() {
        let b = build(ACL_REVISION, &[(0, 0, 0x1000_0000, &world()), (0, 0, 0x1000_0000, &local_system())]);
        assert!(is_valid(&b));
        assert_eq!(header(&b), Some((2, 8 + 20 + 20, 2)));
        assert_eq!(bytes_in_use(&b), Ok(b.len()));
    }
}
