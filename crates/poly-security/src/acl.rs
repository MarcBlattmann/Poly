//! `ACL` and its ACEs (`winnt.h`), on the caller's memory.
//!
//! An ACL is an 8-byte header — `AclRevision` (byte), `Sbz1`, `AclSize`
//! (word, the whole buffer), `AceCount` (word), `Sbz2` — followed by
//! `AceCount` ACEs packed one after the other. Every ACE starts with an
//! `ACE_HEADER` — `AceType`, `AceFlags` (bytes), `AceSize` (word) — and the
//! common ones (`ACCESS_ALLOWED_ACE`, `ACCESS_DENIED_ACE`,
//! `SYSTEM_AUDIT_ACE`, `SYSTEM_MANDATORY_LABEL_ACE`) continue with an
//! `ACCESS_MASK` and the SID (`SidStart`).
//!
//! What the functions do with a damaged ACL, and in which order they check,
//! is what the spec sheets observed (`specs/windows/advapi32.dll/...`):
//! the header's `AclSize` is trusted (an ACL may claim more room than the
//! caller's buffer has), only the bytes the walk touches are read, and the
//! walk follows `AceCount` and each ACE's own size field.
//!
//! All the functions are `unsafe`: they read (and write) the memory the ACL
//! header says belongs to the ACL.

use crate::err::{SecErr, SecResult};
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
pub const ACCESS_ALLOWED_COMPOUND_ACE_TYPE: u8 = 4;
pub const ACCESS_ALLOWED_OBJECT_ACE_TYPE: u8 = 5;
pub const ACCESS_DENIED_OBJECT_ACE_TYPE: u8 = 6;
pub const SYSTEM_AUDIT_OBJECT_ACE_TYPE: u8 = 7;
pub const SYSTEM_ALARM_OBJECT_ACE_TYPE: u8 = 8;
pub const ACCESS_ALLOWED_CALLBACK_ACE_TYPE: u8 = 9;
pub const ACCESS_DENIED_CALLBACK_ACE_TYPE: u8 = 0xA;
pub const ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE: u8 = 0xB;
pub const ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE: u8 = 0xC;
pub const SYSTEM_AUDIT_CALLBACK_ACE_TYPE: u8 = 0xD;
pub const SYSTEM_ALARM_CALLBACK_ACE_TYPE: u8 = 0xE;
pub const SYSTEM_AUDIT_CALLBACK_OBJECT_ACE_TYPE: u8 = 0xF;
pub const SYSTEM_ALARM_CALLBACK_OBJECT_ACE_TYPE: u8 = 0x10;
pub const SYSTEM_MANDATORY_LABEL_ACE_TYPE: u8 = 0x11;
pub const SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE: u8 = 0x12;
pub const SYSTEM_SCOPED_POLICY_ID_ACE_TYPE: u8 = 0x13;

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

/// `SYSTEM_MANDATORY_LABEL_*` policy bits (`winnt.h`).
pub const SYSTEM_MANDATORY_LABEL_NO_WRITE_UP: u32 = 1;
pub const SYSTEM_MANDATORY_LABEL_NO_READ_UP: u32 = 2;
pub const SYSTEM_MANDATORY_LABEL_NO_EXECUTE_UP: u32 = 4;

/// An ACL in memory.
#[derive(Clone, Copy, Debug)]
pub struct AclPtr(pub *mut u8);

impl AclPtr {
    /// # Safety
    /// `off` is inside memory the ACL owns.
    unsafe fn u8(self, off: usize) -> u8 {
        // SAFETY: the caller's contract.
        unsafe { *self.0.add(off) }
    }
    /// # Safety
    /// As [`AclPtr::u8`].
    unsafe fn u16(self, off: usize) -> u16 {
        // SAFETY: the caller's contract.
        unsafe { (self.0.add(off) as *const u16).read_unaligned() }
    }
    /// # Safety
    /// As [`AclPtr::u8`].
    unsafe fn set_u16(self, off: usize, v: u16) {
        // SAFETY: the caller's contract.
        unsafe { (self.0.add(off) as *mut u16).write_unaligned(v) }
    }
    /// `AclRevision`.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn revision(self) -> u8 {
        // SAFETY: the caller upholds this function's contract (see its `# Safety`).
        unsafe { self.u8(0) }
    }
    /// `AclSize`.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn size(self) -> usize {
        // SAFETY: the caller upholds this function's contract (see its `# Safety`).
        usize::from(unsafe { self.u16(2) })
    }
    /// `AceCount`.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn count(self) -> usize {
        // SAFETY: the caller upholds this function's contract (see its `# Safety`).
        usize::from(unsafe { self.u16(4) })
    }
    /// The ACE at byte `off` of the ACL: (type, flags, size).
    ///
    /// # Safety
    /// The four header bytes at `off` are readable.
    pub unsafe fn ace_at(self, off: usize) -> (u8, u8, usize) {
        // SAFETY: the caller upholds this function's contract (see its `# Safety`).
        unsafe { (self.u8(off), self.u8(off + 1), usize::from(self.u16(off + 2))) }
    }
    /// The ACE bytes at `off`, `len` long.
    ///
    /// # Safety
    /// They are readable.
    pub unsafe fn bytes<'a>(self, off: usize, len: usize) -> &'a [u8] {
        // SAFETY: the caller upholds this function's contract (see its `# Safety`).
        unsafe { core::slice::from_raw_parts(self.0.add(off), len) }
    }
    /// The whole ACL as `AclSize` bytes.
    ///
    /// # Safety
    /// They are readable.
    pub unsafe fn all<'a>(self) -> &'a [u8] {
        // SAFETY: the caller upholds this function's contract (see its `# Safety`).
        unsafe { self.bytes(0, self.size()) }
    }
}

/// `InitializeAcl` / `RtlCreateAcl`: write the empty header. A length
/// below 8 is [`SecErr::BufferTooSmall`], one above 0xFFFC or a revision
/// (the whole 32-bit value) other than 2, 3, 4 is [`SecErr::InvalidParameter`].
///
/// # Safety
/// Eight bytes at `acl` are writable.
pub unsafe fn initialize(acl: AclPtr, length: u32, revision: u32) -> SecResult<()> {
    if length < ACL_HEADER as u32 {
        return Err(SecErr::BufferTooSmall);
    }
    if length > 0xFFFC || !(u32::from(MIN_ACL_REVISION)..=u32::from(MAX_ACL_REVISION)).contains(&revision) {
        return Err(SecErr::InvalidParameter);
    }
    // SAFETY: the header is writable.
    unsafe {
        acl.0.write(revision as u8);
        acl.0.add(1).write(0);
        acl.set_u16(2, (length & !3) as u16);
        acl.set_u16(4, 0);
        acl.set_u16(6, 0);
    }
    Ok(())
}

/// Whether an ACE type has a mask and a SID at offset 8 (the layout of
/// `ACCESS_ALLOWED_ACE` and its kin).
const fn access_layout(kind: u8) -> bool {
    matches!(
        kind,
        ACCESS_ALLOWED_ACE_TYPE
            | ACCESS_DENIED_ACE_TYPE
            | SYSTEM_AUDIT_ACE_TYPE
            | SYSTEM_ALARM_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_ACE_TYPE
            | SYSTEM_AUDIT_CALLBACK_ACE_TYPE
            | SYSTEM_ALARM_CALLBACK_ACE_TYPE
            | SYSTEM_MANDATORY_LABEL_ACE_TYPE
            | SYSTEM_SCOPED_POLICY_ID_ACE_TYPE
            | 0x14
            | 0x17
    )
}

/// Whether an ACE type has the object layout (mask, a flags word, up to two
/// GUIDs, the SID) — valid only in a revision 4 ACL.
const fn object_layout(kind: u8) -> bool {
    matches!(kind, 5..=8 | 0xB | 0xC | 0xF | 0x10)
}

/// Check one ACE of `acl` (revision `rev`) at `at`: its header inside the
/// ACL, its size a multiple of 4 and inside, and for the types whose layout
/// is known, a valid SID where the layout puts it.
///
/// # Safety
/// The ACE's bytes are readable.
unsafe fn ace_ok(acl: AclPtr, rev: u8, at: usize, acl_size: usize) -> bool {
    if at + ACE_HEADER > acl_size {
        return false;
    }
    // SAFETY: the header is inside the ACL.
    let (kind, _flags, size) = unsafe { acl.ace_at(at) };
    if size < ACE_HEADER || size % 4 != 0 || at + size > acl_size {
        return false;
    }
    let sid_at = |rel: usize| -> bool {
        if rel + sid::SID_HEADER > size {
            return false;
        }
        // SAFETY: inside the ACE.
        let (r, c) = unsafe { (acl.u8(at + rel), acl.u8(at + rel + 1)) };
        r == sid::SID_REVISION
            && usize::from(c) <= sid::SID_MAX_SUB_AUTHORITIES
            && rel + sid::length_required(c) as usize <= size
    };
    if access_layout(kind) {
        sid_at(ACE_FIXED)
    } else if object_layout(kind) {
        if rev < MAX_ACL_REVISION || size < ACE_FIXED + 4 {
            return false;
        }
        // SAFETY: inside the ACE.
        let flags = unsafe { acl.u16(at + ACE_FIXED) | (acl.u16(at + ACE_FIXED + 2) << 8) };
        let guids = usize::from(flags & 1 != 0) + usize::from(flags & 2 != 0);
        sid_at(ACE_FIXED + 4 + 16 * guids)
    } else {
        // Types 4 (compound), 0x12, 0x15 and 0x16 are not valid here;
        // 0x18 and above are not looked into.
        kind >= 0x18
    }
}

/// `IsValidAcl` / `RtlValidAcl`.
///
/// # Safety
/// The header and the ACEs it leads to are readable.
pub unsafe fn is_valid(acl: AclPtr) -> bool {
    // SAFETY: the header is readable.
    let (rev, size, count) = unsafe { (acl.revision(), acl.size(), acl.count()) };
    if !(MIN_ACL_REVISION..=MAX_ACL_REVISION).contains(&rev) || size < ACL_HEADER {
        return false;
    }
    let mut at = ACL_HEADER;
    for _ in 0..count {
        // SAFETY: `ace_ok` stays inside `size`, which the header vouches for.
        if !unsafe { ace_ok(acl, rev, at, size) } {
            return false;
        }
        // SAFETY: as above.
        at += unsafe { acl.ace_at(at) }.2;
    }
    true
}

/// `RtlFirstFreeAce`: walk `AceCount` ACEs by their sizes. `Err` when the
/// walk starts an ACE outside the ACL; `Ok(None)` when it ends past the
/// ACL's size; else the offset after the last ACE.
///
/// # Safety
/// The walked bytes are readable.
#[allow(clippy::result_unit_err)]
pub unsafe fn first_free(acl: AclPtr) -> Result<Option<usize>, ()> {
    // SAFETY: the header is readable.
    let (size, count) = unsafe { (acl.size(), acl.count()) };
    let mut at = ACL_HEADER;
    for _ in 0..count {
        if at >= size {
            return Err(());
        }
        // SAFETY: `at` is inside the ACL.
        at += unsafe { acl.ace_at(at) }.2;
    }
    Ok((at <= size).then_some(at))
}

/// `GetAclInformation` / `RtlQueryInformationAcl`: the bytes of the
/// structure of `class` (4 or 12 bytes); `length` is the caller's buffer.
/// `strict` (the Win32 function) fails when the ACE walk leaves the ACL;
/// the native one reports what the sizes and the count say, with the bytes
/// in use capped at the ACL's size.
///
/// # Safety
/// The walked bytes are readable.
pub unsafe fn information(acl: AclPtr, class: u32, length: u32, strict: bool) -> SecResult<Vec<u8>> {
    if class != ACL_REVISION_INFORMATION && class != ACL_SIZE_INFORMATION {
        return Err(SecErr::InvalidParameter);
    }
    let need = if class == ACL_REVISION_INFORMATION { 4 } else { 12 };
    if length < need {
        return Err(SecErr::BufferTooSmall);
    }
    // SAFETY: the header is readable.
    let rev = unsafe { acl.revision() };
    if !(MIN_ACL_REVISION..=MAX_ACL_REVISION).contains(&rev) {
        return Err(SecErr::InvalidParameter);
    }
    let mut v = Vec::new();
    if class == ACL_REVISION_INFORMATION {
        v.extend_from_slice(&u32::from(rev).to_le_bytes());
        return Ok(v);
    }
    // SAFETY: the header is readable.
    let (size, count) = unsafe { (acl.size(), acl.count()) };
    let used = if strict {
        // SAFETY: as above.
        match unsafe { first_free(acl) } {
            Ok(Some(u)) => u,
            _ => return Err(SecErr::InvalidParameter),
        }
    } else {
        let mut at = ACL_HEADER;
        for _ in 0..count {
            if at >= size {
                break;
            }
            // SAFETY: inside the ACL.
            at += unsafe { acl.ace_at(at) }.2;
        }
        at.min(size)
    };
    v.extend_from_slice(&(count as u32).to_le_bytes());
    v.extend_from_slice(&(used as u32).to_le_bytes());
    v.extend_from_slice(&((size - used) as u32).to_le_bytes());
    Ok(v)
}

/// `SetAclInformation` / `RtlSetInformationAcl`: only
/// `AclRevisionInformation`; the revision (32 bits) may not be lower than
/// the ACL's, the low byte is stored.
///
/// # Safety
/// The header is writable.
pub unsafe fn set_information(acl: AclPtr, class: u32, info: &[u8]) -> SecResult<()> {
    if class != ACL_REVISION_INFORMATION {
        return Err(SecErr::InvalidParameter);
    }
    if info.len() < 4 {
        return Err(SecErr::BufferTooSmall);
    }
    let want = u32::from_le_bytes([info[0], info[1], info[2], info[3]]);
    // SAFETY: the header is accessible.
    let cur = unsafe { acl.revision() };
    if !(MIN_ACL_REVISION..=MAX_ACL_REVISION).contains(&cur) || want < u32::from(cur) {
        return Err(SecErr::InvalidParameter);
    }
    // SAFETY: as above.
    unsafe { acl.0.write(want as u8) };
    Ok(())
}

/// `GetAce` / `RtlGetAce`: the offset of ACE `index`. The count limits the
/// index and the revision has to be 2..4 (`Err((_, None))`: the caller's
/// out pointer stays as it was); the ACE sizes decide where the ACE is,
/// they are not validated; the pointer is stored even when the ACE would not
/// start inside the ACL, which is then an error too (`Err((_, Some(off)))`).
///
/// # Safety
/// The walked bytes are readable.
pub unsafe fn ace_offset(acl: AclPtr, index: u32) -> Result<usize, (SecErr, Option<usize>)> {
    // SAFETY: the header is readable.
    let (rev, size, count) = unsafe { (acl.revision(), acl.size(), acl.count()) };
    if !(MIN_ACL_REVISION..=MAX_ACL_REVISION).contains(&rev) || index as usize >= count {
        return Err((SecErr::InvalidParameter, None));
    }
    let mut at = ACL_HEADER;
    for _ in 0..index {
        if at >= size {
            break;
        }
        // SAFETY: inside the ACL.
        at += unsafe { acl.ace_at(at) }.2;
    }
    if at >= size {
        return Err((SecErr::InvalidParameter, Some(at)));
    }
    Ok(at)
}

/// `DeleteAce` / `RtlDeleteAce`: the ACL has to be valid first (the whole
/// list), the later ACEs move down and the freed bytes at the end of the
/// used area are zeroed.
///
/// # Safety
/// The ACL's bytes are readable and writable.
pub unsafe fn delete(acl: AclPtr, index: u32) -> SecResult<()> {
    // SAFETY: as documented.
    unsafe {
        if !is_valid(acl) || index as usize >= acl.count() {
            return Err(SecErr::InvalidParameter);
        }
        let at = ace_offset(acl, index).map_err(|e| e.0)?;
        let len = acl.ace_at(at).2;
        let used = match first_free(acl) {
            Ok(Some(u)) => u,
            _ => return Err(SecErr::InvalidParameter),
        };
        core::ptr::copy(acl.0.add(at + len), acl.0.add(at), used - at - len);
        core::ptr::write_bytes(acl.0.add(used - len), 0, len);
        let count = acl.u16(4) - 1;
        acl.set_u16(4, count);
    }
    Ok(())
}

/// The revision an ACL has after an ACE needing `revision` is added: the
/// larger of the two (the low byte of the argument, Learn's `AddAce`).
fn raised(current: u8, revision: u32) -> u8 {
    current.max(revision as u8)
}

/// `AddAce` / `RtlAddAce`: insert the whole ACEs in `list` before ACE
/// `start` (at the end for an index at or past the count). The ACL has to
/// have a revision of 2..4, a walkable ACE list and room; the list has to be
/// exactly whole ACEs; types 4 to 8 need `revision` 3 or 4 (Learn: object
/// ACEs need revision 4).
///
/// # Safety
/// The ACL's bytes are readable and writable.
pub unsafe fn insert(acl: AclPtr, revision: u32, start: u32, list: &[u8]) -> SecResult<()> {
    // SAFETY: as documented.
    unsafe {
        let (rev, size) = (acl.revision(), acl.size());
        if !(MIN_ACL_REVISION..=MAX_ACL_REVISION).contains(&rev) || size < ACL_HEADER {
            return Err(SecErr::InvalidParameter);
        }
        if !is_valid(acl) {
            return Err(SecErr::InvalidParameter);
        }
        let used = match first_free(acl) {
            Ok(Some(u)) => u,
            _ => return Err(SecErr::InvalidParameter),
        };
        // The list: whole ACEs, the types the revision argument allows.
        let mut at = 0usize;
        let mut n = 0u32;
        while at < list.len() {
            if at + ACE_HEADER > list.len() {
                return Err(SecErr::InvalidParameter);
            }
            let size_field = usize::from(u16::from_le_bytes([list[at + 2], list[at + 3]]));
            if size_field < ACE_HEADER || at + size_field > list.len() {
                return Err(SecErr::InvalidParameter);
            }
            let kind = list[at];
            if (kind == ACCESS_ALLOWED_COMPOUND_ACE_TYPE && revision < 3) || ((5..=8).contains(&kind) && revision < 4) {
                return Err(SecErr::InvalidParameter);
            }
            at += size_field;
            n += 1;
        }
        if used + list.len() > size {
            return Err(SecErr::BufferTooSmall);
        }
        // Where the first of them goes.
        let mut pos = ACL_HEADER;
        let count = acl.count();
        let mut i = 0usize;
        while i < count && i < start as usize {
            pos += acl.ace_at(pos).2;
            i += 1;
        }
        if i >= count || (start as usize) >= count {
            pos = used;
        }
        core::ptr::copy(acl.0.add(pos), acl.0.add(pos + list.len()), used - pos);
        core::ptr::copy_nonoverlapping(list.as_ptr(), acl.0.add(pos), list.len());
        acl.set_u16(4, (count as u32 + n) as u16);
        acl.0.write(raised(rev, revision));
    }
    Ok(())
}

/// One ACE with a mask and a SID.
pub fn ace(kind: u8, flags: u8, mask: u32, sid: &[u8]) -> Result<Vec<u8>, SecErr> {
    let len = sid::valid_length(sid).ok_or(SecErr::InvalidSid)?;
    let size = ACE_FIXED + len;
    let mut v = Vec::with_capacity(size);
    v.extend_from_slice(&[kind, flags]);
    v.extend_from_slice(&(size as u16).to_le_bytes());
    v.extend_from_slice(&mask.to_le_bytes());
    v.extend_from_slice(&sid[..len]);
    Ok(v)
}

/// `AddAccessAllowedAce(Ex)`, `AddAccessDeniedAce(Ex)`,
/// `AddAuditAccessAce(Ex)`, `AddMandatoryAce`: append one ACE (the caller
/// has checked the flags). The revision argument (the whole 32 bits) above 4
/// is [`SecErr::RevisionMismatch`], as is an ACL revision above 4; an ACL
/// revision below 2, a header smaller than 8 or an ACE list that cannot be
/// walked is [`SecErr::InvalidAcl`]; no room is [`SecErr::AllottedSpace`].
///
/// # Safety
/// The ACL's bytes are readable and writable.
pub unsafe fn add(acl: AclPtr, revision: u32, kind: u8, flags: u8, mask: u32, sid: &[u8]) -> SecResult<()> {
    if revision > u32::from(MAX_ACL_REVISION) {
        return Err(SecErr::RevisionMismatch);
    }
    // SAFETY: as documented.
    unsafe {
        let (rev, size) = (acl.revision(), acl.size());
        if rev > MAX_ACL_REVISION {
            return Err(SecErr::RevisionMismatch);
        }
        if rev < MIN_ACL_REVISION || size < ACL_HEADER {
            return Err(SecErr::InvalidAcl);
        }
        if !is_valid(acl) {
            return Err(SecErr::InvalidAcl);
        }
        let new = ace(kind, flags, mask, sid)?;
        let used = match first_free(acl) {
            Ok(Some(u)) => u,
            _ => return Err(SecErr::InvalidAcl),
        };
        if used + new.len() > size {
            return Err(SecErr::AllottedSpace);
        }
        core::ptr::copy_nonoverlapping(new.as_ptr(), acl.0.add(used), new.len());
        let count = acl.count() + 1;
        acl.set_u16(4, count as u16);
        acl.0.write(raised(rev, revision));
    }
    Ok(())
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

/// One ACE of an ACL as a value, for the access check and the SDDL writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ace {
    pub kind: u8,
    pub flags: u8,
    /// The whole ACE (header included).
    pub bytes: Vec<u8>,
}

impl Ace {
    /// The mask (offset 4) of an ACE with a mask.
    pub fn mask(&self) -> u32 {
        u32::from_le_bytes([self.bytes[4], self.bytes[5], self.bytes[6], self.bytes[7]])
    }

    /// The SID of an access-layout ACE (offset 8), or of an object ACE.
    pub fn sid(&self) -> Option<&[u8]> {
        let at = if object_layout(self.kind) {
            let flags = u32::from_le_bytes([self.bytes[8], self.bytes[9], self.bytes[10], self.bytes[11]]);
            12 + 16 * (usize::from(flags & 1 != 0) + usize::from(flags & 2 != 0))
        } else {
            ACE_FIXED
        };
        self.bytes.get(at..)
    }
}

/// The ACEs of an ACL (the walk follows the sizes; stops at the first ACE
/// that does not fit).
///
/// # Safety
/// The ACL's bytes are readable.
pub unsafe fn aces(acl: AclPtr) -> Vec<Ace> {
    // SAFETY: as documented.
    unsafe {
        let (size, count) = (acl.size(), acl.count());
        let mut out = Vec::new();
        let mut at = ACL_HEADER;
        for _ in 0..count {
            if at + ACE_HEADER > size {
                break;
            }
            let (kind, flags, len) = acl.ace_at(at);
            if len < ACE_HEADER || at + len > size {
                break;
            }
            out.push(Ace { kind, flags, bytes: acl.bytes(at, len).to_vec() });
            at += len;
        }
        out
    }
}

/// The ACEs of an ACL held in `b` (header and ACEs): the walk follows the
/// header's count and the ACEs' sizes, within `b`, whatever `AclSize` says.
pub fn aces_in(b: &[u8]) -> Vec<Ace> {
    let mut out = Vec::new();
    if b.len() < ACL_HEADER {
        return out;
    }
    let count = usize::from(u16::from_le_bytes([b[4], b[5]]));
    let mut at = ACL_HEADER;
    for _ in 0..count {
        if at + ACE_HEADER > b.len() {
            break;
        }
        let len = usize::from(u16::from_le_bytes([b[at + 2], b[at + 3]]));
        if len < ACE_HEADER || at + len > b.len() {
            break;
        }
        out.push(Ace { kind: b[at], flags: b[at + 1], bytes: b[at..at + len].to_vec() });
        at += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sid::{builtin, local_system, world};

    fn acl(len: usize) -> Vec<u8> {
        let mut b = alloc::vec![0xCCu8; len + 64];
        // SAFETY: the buffer is larger than the length.
        unsafe { initialize(AclPtr(b.as_mut_ptr()), len as u32, 2).unwrap() };
        b
    }
    fn p(b: &mut Vec<u8>) -> AclPtr {
        AclPtr(b.as_mut_ptr())
    }

    #[test]
    fn initialize_rules() {
        let mut b = alloc::vec![0xCCu8; 128];
        // SAFETY: 128 bytes.
        unsafe {
            assert_eq!(initialize(p(&mut b), 4, 2), Err(SecErr::BufferTooSmall));
            assert_eq!(initialize(p(&mut b), 64, 1), Err(SecErr::InvalidParameter));
            assert_eq!(initialize(p(&mut b), 64, 0x10002), Err(SecErr::InvalidParameter));
            assert_eq!(initialize(p(&mut b), 65533, 2), Err(SecErr::InvalidParameter));
            assert_eq!(b[0], 0xCC);
            initialize(p(&mut b), 62, 4).unwrap();
            assert_eq!(&b[..8], &[4, 0, 60, 0, 0, 0, 0, 0]);
            assert_eq!(b[8], 0xCC);
            assert!(is_valid(p(&mut b)));
            assert_eq!(information(p(&mut b), 2, 12, true).unwrap(), [0, 0, 0, 0, 8, 0, 0, 0, 52, 0, 0, 0]);
            assert_eq!(information(p(&mut b), 1, 4, true).unwrap(), [4, 0, 0, 0]);
            assert_eq!(information(p(&mut b), 3, 12, true), Err(SecErr::InvalidParameter));
            assert_eq!(information(p(&mut b), 2, 11, true), Err(SecErr::BufferTooSmall));
        }
    }

    #[test]
    fn add_get_insert_delete() {
        let everyone = world().to_bytes();
        let admins = builtin(544).to_bytes();
        let mut b = acl(8 + 20 + 24 + 20);
        // SAFETY: the buffer holds the ACL and more.
        unsafe {
            add(p(&mut b), 2, 0, 0, 0x1F_01FF, &everyone).unwrap();
            add(p(&mut b), 2, 1, CONTAINER_INHERIT_ACE, 0x10000, &admins).unwrap();
            assert_eq!(add(p(&mut b), 2, 0, 0, 1, &everyone), Ok(()));
            assert_eq!(add(p(&mut b), 2, 0, 0, 1, &everyone), Err(SecErr::AllottedSpace));
            assert_eq!(ace_offset(p(&mut b), 0), Ok(8));
            assert_eq!(ace_offset(p(&mut b), 1), Ok(28));
            assert_eq!(ace_offset(p(&mut b), 2), Ok(52));
            assert_eq!(ace_offset(p(&mut b), 3), Err((SecErr::InvalidParameter, None)));
            assert_eq!(&b[28..32], &[1, 2, 24, 0]);
            assert_eq!(first_free(p(&mut b)), Ok(Some(72)));
            // Delete the first and the rest move down; the tail is zeroed.
            delete(p(&mut b), 0).unwrap();
            assert_eq!(&b[8..12], &[1, 2, 24, 0]);
            assert!(b[52..72].iter().all(|&x| x == 0));
            assert_eq!(b[72], 0xCC);
            // Insert one at the front with AddAce.
            let e = ace(0, 0, 5, &everyone).unwrap();
            insert(p(&mut b), 2, 0, &e).unwrap();
            assert_eq!(&b[8..20], &e[..12]);
            assert!(is_valid(p(&mut b)));
            // A list that is not whole ACEs; no room.
            assert_eq!(insert(p(&mut b), 2, 0, &e[..19]), Err(SecErr::InvalidParameter));
            assert_eq!(insert(p(&mut b), 2, 0xFFFF_FFFF, &e), Err(SecErr::BufferTooSmall));
            // Types 4..8 need the revision.
            let mut o = e.clone();
            o[0] = 5;
            assert_eq!(insert(p(&mut b), 3, 0, &o), Err(SecErr::InvalidParameter));
        }
    }

    #[test]
    fn argument_errors() {
        let s = world().to_bytes();
        let mut b = acl(64);
        // SAFETY: the buffer is large enough.
        unsafe {
            assert_eq!(add(p(&mut b), 5, 0, 0, 1, &s), Err(SecErr::RevisionMismatch));
            assert_eq!(add(p(&mut b), 0x10002, 0, 0, 1, &s), Err(SecErr::RevisionMismatch));
            assert_eq!(add(p(&mut b), 2, 0, 0, 1, &[2, 0, 0, 0, 0, 0, 0, 0]), Err(SecErr::InvalidSid));
            add(p(&mut b), 4, 0, 0, 1, &s).unwrap();
            assert_eq!(b[0], 4);
            add(p(&mut b), 2, 0, 0, 1, &s).unwrap();
            assert_eq!(b[0], 4);
            let mut bad = acl(16);
            bad[0] = 7;
            assert_eq!(add(p(&mut bad), 2, 0, 0, 1, &s), Err(SecErr::RevisionMismatch));
            bad[0] = 1;
            assert_eq!(add(p(&mut bad), 2, 0, 0, 1, &s), Err(SecErr::InvalidAcl));
            let mut broken = acl(16);
            broken[4] = 1; // one ACE claimed, none there (0xCC bytes follow)
            assert_eq!(add(p(&mut broken), 2, 0, 0, 1, &s), Err(SecErr::InvalidAcl));
            assert!(!is_valid(p(&mut broken)));
            assert_eq!(delete(p(&mut acl(16)), 0), Err(SecErr::InvalidParameter));
        }
    }

    #[test]
    fn set_information_rules() {
        let mut b = acl(32);
        // SAFETY: as above.
        unsafe {
            assert_eq!(set_information(p(&mut b), 1, &[0, 0, 0]), Err(SecErr::BufferTooSmall));
            assert_eq!(set_information(p(&mut b), 2, &[3, 0, 0, 0]), Err(SecErr::InvalidParameter));
            set_information(p(&mut b), 1, &[3, 0, 0, 0]).unwrap();
            assert_eq!(b[0], 3);
            assert_eq!(set_information(p(&mut b), 1, &[2, 0, 0, 0]), Err(SecErr::InvalidParameter));
            set_information(p(&mut b), 1, &[4, 1, 0, 0]).unwrap();
            assert_eq!(b[0], 4);
            set_information(p(&mut b), 1, &[0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
            assert_eq!(b[0], 0xFF);
        }
    }

    #[test]
    fn built_acls_are_valid() {
        let mut b = build(ACL_REVISION, &[(0, 0, 0x1000_0000, &world()), (0, 0, 0x1000_0000, &local_system())]);
        // SAFETY: exactly the ACL.
        unsafe {
            assert!(is_valid(p(&mut b)));
            assert_eq!(first_free(p(&mut b)), Ok(Some(b.len())));
            let a = aces(p(&mut b));
            assert_eq!(a.len(), 2);
            assert_eq!(a[0].mask(), 0x1000_0000);
            assert_eq!(a[1].sid().map(|s| s.to_vec()), Some(local_system().to_bytes()));
        }
    }

    #[test]
    fn validity_walk() {
        let mut b = build(ACL_REVISION, &[(0, 0, 1, &world()), (1, 0, 2, &builtin(544))]);
        // SAFETY: exactly the ACL (plus the zero bytes of the test).
        unsafe {
            b.resize(b.len() + 64, 0xCC);
            // ACL size field larger than the buffer is fine; a count that is too big is not.
            assert!(is_valid(p(&mut b)));
            b[4] = 3;
            assert!(!is_valid(p(&mut b)));
            b[4] = 2;
            // The second ACE's size not a multiple of 4.
            b[29] = 0;
            let keep = b[30];
            b[30] = 23;
            assert!(!is_valid(p(&mut b)));
            b[30] = keep;
            // An object-layout type only in revision 4.
            b[8] = 5;
            assert!(!is_valid(p(&mut b)));
        }
    }
}
