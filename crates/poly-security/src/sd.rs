//! Security descriptors (`winnt.h`).
//!
//! The absolute form, `SECURITY_DESCRIPTOR`, on x64: `Revision` (byte),
//! `Sbz1`, `Control` (word), then four pointers at 8, 16, 24 and 32 —
//! `Owner`, `Group`, `Sacl`, `Dacl` — 40 bytes
//! (`SECURITY_DESCRIPTOR_MIN_LENGTH`). The self-relative form,
//! `SECURITY_DESCRIPTOR_RELATIVE`, has `SE_SELF_RELATIVE` in its control
//! and four `DWORD` offsets from its start at 4, 8, 12 and 16 (0: absent),
//! 20 bytes, followed by the parts: `MakeSelfRelativeSD` writes the SACL, the
//! DACL, the owner and the group in that order with no padding (spec
//! sheet).
//!
//! The functions work on the caller's memory (`unsafe`): the slots and the
//! parts they lead to are trusted the way Windows trusts them, the spec
//! sheets say what is checked and what is not.

use crate::acl::{self, AclPtr};
use crate::err::{SecErr, SecResult};
use crate::sid;
use alloc::vec::Vec;

/// `SECURITY_DESCRIPTOR_REVISION`.
pub const SECURITY_DESCRIPTOR_REVISION: u8 = 1;
/// `SECURITY_DESCRIPTOR_MIN_LENGTH` (`sizeof(SECURITY_DESCRIPTOR)`, x64).
pub const SECURITY_DESCRIPTOR_MIN_LENGTH: usize = 40;
/// `sizeof(SECURITY_DESCRIPTOR_RELATIVE)`.
pub const RELATIVE_HEADER: usize = 20;
/// Offsets of the absolute form's pointers.
pub const ABS_OWNER: usize = 8;
pub const ABS_GROUP: usize = 16;
pub const ABS_SACL: usize = 24;
pub const ABS_DACL: usize = 32;
/// Offsets of the self-relative form's offsets.
pub const REL_OWNER: usize = 4;
pub const REL_GROUP: usize = 8;
pub const REL_SACL: usize = 12;
pub const REL_DACL: usize = 16;

/// `SECURITY_DESCRIPTOR_CONTROL` bits (`winnt.h`).
pub const SE_OWNER_DEFAULTED: u16 = 0x0001;
pub const SE_GROUP_DEFAULTED: u16 = 0x0002;
pub const SE_DACL_PRESENT: u16 = 0x0004;
pub const SE_DACL_DEFAULTED: u16 = 0x0008;
pub const SE_SACL_PRESENT: u16 = 0x0010;
pub const SE_SACL_DEFAULTED: u16 = 0x0020;
pub const SE_DACL_UNTRUSTED: u16 = 0x0040;
pub const SE_SERVER_SECURITY: u16 = 0x0080;
pub const SE_DACL_AUTO_INHERIT_REQ: u16 = 0x0100;
pub const SE_SACL_AUTO_INHERIT_REQ: u16 = 0x0200;
pub const SE_DACL_AUTO_INHERITED: u16 = 0x0400;
pub const SE_SACL_AUTO_INHERITED: u16 = 0x0800;
pub const SE_DACL_PROTECTED: u16 = 0x1000;
pub const SE_SACL_PROTECTED: u16 = 0x2000;
pub const SE_RM_CONTROL_VALID: u16 = 0x4000;
pub const SE_SELF_RELATIVE: u16 = 0x8000;

/// The bits `SetSecurityDescriptorControl` may change (spec sheet: the eight
/// bits 0x0040 to 0x2000).
pub const SETTABLE_CONTROL: u16 = SE_DACL_UNTRUSTED
    | SE_SERVER_SECURITY
    | SE_DACL_AUTO_INHERIT_REQ
    | SE_SACL_AUTO_INHERIT_REQ
    | SE_DACL_AUTO_INHERITED
    | SE_SACL_AUTO_INHERITED
    | SE_DACL_PROTECTED
    | SE_SACL_PROTECTED;

/// `SECURITY_INFORMATION` bits (`winnt.h`).
pub const OWNER_SECURITY_INFORMATION: u32 = 0x1;
pub const GROUP_SECURITY_INFORMATION: u32 = 0x2;
pub const DACL_SECURITY_INFORMATION: u32 = 0x4;
pub const SACL_SECURITY_INFORMATION: u32 = 0x8;
pub const LABEL_SECURITY_INFORMATION: u32 = 0x10;
pub const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
pub const PROTECTED_SACL_SECURITY_INFORMATION: u32 = 0x4000_0000;
pub const UNPROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x2000_0000;
pub const UNPROTECTED_SACL_SECURITY_INFORMATION: u32 = 0x1000_0000;

/// A security descriptor in memory.
#[derive(Clone, Copy, Debug)]
pub struct SdPtr(pub *mut u8);

fn plausible(p: u64) -> bool {
    (0x1_0000..=0x7FFF_FFFF_FFFF).contains(&p)
}

impl SdPtr {
    /// `Revision`.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn revision(self) -> u8 {
        unsafe { *self.0 }
    }
    /// `Control`.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn control(self) -> u16 {
        unsafe { (self.0.add(2) as *const u16).read_unaligned() }
    }
    /// # Safety
    /// The header is writable.
    pub unsafe fn set_control_word(self, c: u16) {
        unsafe { (self.0.add(2) as *mut u16).write_unaligned(c) }
    }
    /// Whether `SE_SELF_RELATIVE` is set.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn is_relative(self) -> bool {
        unsafe { self.control() & SE_SELF_RELATIVE != 0 }
    }
    /// The part at the absolute slot `abs` (or relative offset `rel`):
    /// null for none.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn part(self, abs: usize, rel: usize) -> *mut u8 {
        unsafe {
            if self.is_relative() {
                let off = (self.0.add(rel) as *const u32).read_unaligned();
                if off == 0 { core::ptr::null_mut() } else { self.0.add(off as usize) }
            } else {
                (self.0.add(abs) as *const *mut u8).read_unaligned()
            }
        }
    }
    /// # Safety
    /// The absolute header is writable.
    pub unsafe fn set_slot(self, abs: usize, p: *mut u8) {
        unsafe { (self.0.add(abs) as *mut *mut u8).write_unaligned(p) }
    }
    /// The owner SID pointer (null for none).
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn owner(self) -> *mut u8 {
        unsafe { self.part(ABS_OWNER, REL_OWNER) }
    }
    /// The group SID pointer.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn group(self) -> *mut u8 {
        unsafe { self.part(ABS_GROUP, REL_GROUP) }
    }
    /// The SACL pointer as stored (null for none), whether present or not.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn sacl_slot(self) -> *mut u8 {
        unsafe { self.part(ABS_SACL, REL_SACL) }
    }
    /// The DACL pointer as stored.
    ///
    /// # Safety
    /// The header is readable.
    pub unsafe fn dacl_slot(self) -> *mut u8 {
        unsafe { self.part(ABS_DACL, REL_DACL) }
    }
}

/// `InitializeSecurityDescriptor` / `RtlCreateSecurityDescriptor`.
///
/// # Safety
/// 40 bytes at `sd` are writable.
pub unsafe fn create(sd: SdPtr, revision: u32) -> SecResult<()> {
    if revision != u32::from(SECURITY_DESCRIPTOR_REVISION) {
        return Err(SecErr::UnknownRevision);
    }
    // SAFETY: as documented.
    unsafe {
        core::ptr::write_bytes(sd.0, 0, SECURITY_DESCRIPTOR_MIN_LENGTH);
        sd.0.write(SECURITY_DESCRIPTOR_REVISION);
    }
    Ok(())
}

/// What every `Set*` function checks on the descriptor first: revision 1
/// ([`SecErr::UnknownRevision`]) and absolute ([`SecErr::InvalidSd`]).
///
/// # Safety
/// The header is readable.
pub unsafe fn check_absolute(sd: SdPtr) -> SecResult<()> {
    // SAFETY: as documented.
    unsafe {
        if sd.revision() != SECURITY_DESCRIPTOR_REVISION {
            Err(SecErr::UnknownRevision)
        } else if sd.is_relative() {
            Err(SecErr::InvalidSd)
        } else {
            Ok(())
        }
    }
}

/// `SetSecurityDescriptorOwner` / `...Group`: store the pointer and the
/// defaulted bit.
///
/// # Safety
/// The descriptor is writable.
pub unsafe fn set_owner_or_group(sd: SdPtr, group: bool, sid: *mut u8, defaulted: bool) -> SecResult<()> {
    // SAFETY: as documented.
    unsafe {
        check_absolute(sd)?;
        let (slot, bit) = if group { (ABS_GROUP, SE_GROUP_DEFAULTED) } else { (ABS_OWNER, SE_OWNER_DEFAULTED) };
        sd.set_slot(slot, sid);
        let c = sd.control() & !bit;
        sd.set_control_word(if defaulted { c | bit } else { c });
    }
    Ok(())
}

/// `SetSecurityDescriptorDacl` / `...Sacl`: with `present` the slot gets the
/// pointer and `SE_*_PRESENT` is set, `SE_*_DEFAULTED` follows `defaulted`;
/// without, only the present bit is cleared (spec sheet).
///
/// # Safety
/// The descriptor is writable.
pub unsafe fn set_acl(sd: SdPtr, sacl: bool, present: bool, acl: *mut u8, defaulted: bool) -> SecResult<()> {
    // SAFETY: as documented.
    unsafe {
        check_absolute(sd)?;
        let (slot, pbit, dbit) = if sacl {
            (ABS_SACL, SE_SACL_PRESENT, SE_SACL_DEFAULTED)
        } else {
            (ABS_DACL, SE_DACL_PRESENT, SE_DACL_DEFAULTED)
        };
        let mut c = sd.control();
        if !present {
            c &= !pbit;
        } else {
            sd.set_slot(slot, acl);
            c = (c | pbit) & !dbit;
            if defaulted {
                c |= dbit;
            }
        }
        sd.set_control_word(c);
    }
    Ok(())
}

/// What `GetSecurityDescriptorDacl` / `...Sacl` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AclInfo {
    pub present: bool,
    /// Only meaningful when present.
    pub acl: *mut u8,
    pub defaulted: bool,
}

/// `Get*Dacl` / `Get*Sacl`: revision 1 is required; an absent ACL reports
/// not present (the pointer is not for the caller to use).
///
/// # Safety
/// The header is readable.
pub unsafe fn get_acl(sd: SdPtr, sacl: bool) -> SecResult<AclInfo> {
    // SAFETY: as documented.
    unsafe {
        if sd.revision() != SECURITY_DESCRIPTOR_REVISION {
            return Err(SecErr::UnknownRevision);
        }
        let (pbit, dbit) =
            if sacl { (SE_SACL_PRESENT, SE_SACL_DEFAULTED) } else { (SE_DACL_PRESENT, SE_DACL_DEFAULTED) };
        let c = sd.control();
        if c & pbit == 0 {
            return Ok(AclInfo { present: false, acl: core::ptr::null_mut(), defaulted: false });
        }
        let p = if sacl { sd.sacl_slot() } else { sd.dacl_slot() };
        Ok(AclInfo { present: true, acl: p, defaulted: c & dbit != 0 })
    }
}

/// `GetSecurityDescriptorOwner` / `...Group`: the pointer and the defaulted
/// bit; revision 1 required.
///
/// # Safety
/// The header is readable.
pub unsafe fn get_owner_or_group(sd: SdPtr, group: bool) -> SecResult<(*mut u8, bool)> {
    // SAFETY: as documented.
    unsafe {
        if sd.revision() != SECURITY_DESCRIPTOR_REVISION {
            return Err(SecErr::UnknownRevision);
        }
        let (p, bit) = if group { (sd.group(), SE_GROUP_DEFAULTED) } else { (sd.owner(), SE_OWNER_DEFAULTED) };
        Ok((p, sd.control() & bit != 0))
    }
}

/// `SetSecurityDescriptorControl`: change the `mask` bits to `bits`; no
/// revision check. The mask has to be non-empty and inside
/// [`SETTABLE_CONTROL`], `bits` inside the mask.
///
/// # Safety
/// The header is writable.
pub unsafe fn set_control(sd: SdPtr, mask: u16, bits: u16) -> SecResult<()> {
    if mask == 0 || mask & !SETTABLE_CONTROL != 0 || bits & !mask != 0 {
        return Err(SecErr::InvalidParameter);
    }
    // SAFETY: as documented.
    unsafe {
        let c = sd.control();
        sd.set_control_word((c & !mask) | bits);
    }
    Ok(())
}

/// The length of the SID at `p`.
///
/// # Safety
/// The count byte is readable.
unsafe fn sid_len(p: *const u8) -> usize {
    unsafe { sid::length_required(*p.add(1)) as usize }
}

/// `GetSecurityDescriptorLength` / `RtlLengthSecurityDescriptor`: the header
/// plus the parts (the DACL and SACL only when present), nothing validated.
/// The header is 20 bytes for a self-relative descriptor, 40 otherwise.
///
/// # Safety
/// The parts are readable.
pub unsafe fn length(sd: SdPtr) -> usize {
    // SAFETY: as documented.
    unsafe {
        let rel = sd.is_relative();
        let mut n = if rel { RELATIVE_HEADER } else { SECURITY_DESCRIPTOR_MIN_LENGTH };
        let c = sd.control();
        for p in [sd.owner(), sd.group()] {
            if !p.is_null() {
                n += sid_len(p);
            }
        }
        if c & SE_SACL_PRESENT != 0 && !sd.sacl_slot().is_null() {
            n += (AclPtr(sd.sacl_slot()).size() + 3) & !3;
        }
        if c & SE_DACL_PRESENT != 0 && !sd.dacl_slot().is_null() {
            n += (AclPtr(sd.dacl_slot()).size() + 3) & !3;
        }
        n
    }
}

/// `IsValidSecurityDescriptor` / `RtlValidSecurityDescriptor`: revision 1;
/// aligned offsets in a self-relative one; valid SIDs for the owner and the
/// group; valid ACLs for a present DACL and SACL.
///
/// # Safety
/// The parts the probe `readable(pointer, length)` accepts are readable
/// (anything else counts as invalid without being read).
pub unsafe fn is_valid(sd: SdPtr, readable: &dyn Fn(*const u8, usize) -> bool) -> bool {
    // SAFETY: as documented.
    unsafe {
        if !readable(sd.0, RELATIVE_HEADER) || sd.revision() != SECURITY_DESCRIPTOR_REVISION {
            return false;
        }
        let rel = sd.is_relative();
        let c = sd.control();
        let check_addr = |p: *mut u8, n: usize| -> bool { plausible(p as u64) && readable(p, n) };
        if rel {
            for at in [REL_OWNER, REL_GROUP, REL_SACL, REL_DACL] {
                let off = (sd.0.add(at) as *const u32).read_unaligned();
                if off % 4 != 0 {
                    return false;
                }
            }
        }
        for p in [sd.owner(), sd.group()] {
            if p.is_null() {
                continue;
            }
            if !check_addr(p, sid::SID_HEADER) {
                return false;
            }
            let (r, n) = (*p, *p.add(1));
            if r != sid::SID_REVISION
                || usize::from(n) > sid::SID_MAX_SUB_AUTHORITIES
                || !readable(p, sid::length_required(n) as usize)
            {
                return false;
            }
        }
        for (present, p) in [(SE_SACL_PRESENT, sd.sacl_slot()), (SE_DACL_PRESENT, sd.dacl_slot())] {
            if c & present == 0 || p.is_null() {
                continue;
            }
            if !check_addr(p, acl::ACL_HEADER)
                || !readable(p, AclPtr(p).size().max(acl::ACL_HEADER))
                || !acl::is_valid(AclPtr(p))
            {
                return false;
            }
        }
        true
    }
}

/// A descriptor's parts as values: the control (without the
/// self-relative bit), the SIDs and the ACLs as bytes. The presence bits
/// are in `control`; `dacl: None` with `SE_DACL_PRESENT` is a NULL DACL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parts {
    pub revision: u8,
    pub sbz1: u8,
    pub control: u16,
    pub owner: Option<Vec<u8>>,
    pub group: Option<Vec<u8>>,
    pub sacl: Option<Vec<u8>>,
    pub dacl: Option<Vec<u8>>,
}

impl Parts {
    /// Read an absolute or self-relative descriptor.
    ///
    /// # Safety
    /// The parts are readable.
    pub unsafe fn read(sd: SdPtr) -> Parts {
        // SAFETY: as documented.
        unsafe {
            let c = sd.control();
            let sid_of = |p: *mut u8| -> Option<Vec<u8>> {
                (!p.is_null()).then(|| core::slice::from_raw_parts(p, sid_len(p)).to_vec())
            };
            let acl_of = |p: *mut u8| -> Option<Vec<u8>> {
                (!p.is_null()).then(|| {
                    // The ACL as far as its size field or its ACEs go.
                    let a = AclPtr(p);
                    let mut end = a.size().max(acl::ACL_HEADER);
                    let mut at = acl::ACL_HEADER;
                    for _ in 0..a.count() {
                        if at >= 0x1_0000 {
                            break;
                        }
                        let n = a.ace_at(at).2;
                        if n < acl::ACE_HEADER {
                            break;
                        }
                        at += n;
                        end = end.max(at);
                    }
                    core::slice::from_raw_parts(p, end.min(0x1_0000)).to_vec()
                })
            };
            Parts {
                revision: sd.revision(),
                sbz1: *sd.0.add(1),
                control: c & !SE_SELF_RELATIVE,
                owner: sid_of(sd.owner()),
                group: sid_of(sd.group()),
                sacl: if c & SE_SACL_PRESENT != 0 { acl_of(sd.sacl_slot()) } else { None },
                dacl: if c & SE_DACL_PRESENT != 0 { acl_of(sd.dacl_slot()) } else { None },
            }
        }
    }

    /// The self-relative form's length.
    pub fn relative_length(&self) -> usize {
        RELATIVE_HEADER
            + [&self.sacl, &self.dacl, &self.owner, &self.group].into_iter().flatten().map(Vec::len).sum::<usize>()
    }

    /// The self-relative form: the 20-byte header, then the SACL, DACL,
    /// owner and group in that order with no padding.
    pub fn to_relative(&self) -> Vec<u8> {
        let mut v = alloc::vec![0u8; RELATIVE_HEADER];
        v[0] = self.revision;
        v[1] = self.sbz1;
        v[2..4].copy_from_slice(&(self.control | SE_SELF_RELATIVE).to_le_bytes());
        for (at, part) in
            [(REL_SACL, &self.sacl), (REL_DACL, &self.dacl), (REL_OWNER, &self.owner), (REL_GROUP, &self.group)]
        {
            if let Some(p) = part {
                let off = v.len() as u32;
                v[at..at + 4].copy_from_slice(&off.to_le_bytes());
                v.extend_from_slice(p);
            }
        }
        v
    }

    /// Parts from a self-relative descriptor's bytes (the offsets stay
    /// inside `b`); `None` if they do not.
    pub fn from_relative(b: &[u8]) -> Option<Parts> {
        if b.len() < RELATIVE_HEADER {
            return None;
        }
        let c = u16::from_le_bytes([b[2], b[3]]);
        let dword = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize;
        let sid_at = |at: usize| -> Option<Option<Vec<u8>>> {
            match dword(at) {
                0 => Some(None),
                off if off >= b.len() => None,
                off => sid::valid_length(&b[off..]).map(|l| Some(b[off..off + l].to_vec())),
            }
        };
        let acl_at = |at: usize, present: bool| -> Option<Option<Vec<u8>>> {
            match dword(at) {
                0 => Some(None),
                _ if !present => Some(None),
                off if off + acl::ACL_HEADER > b.len() => None,
                off => {
                    let size = usize::from(u16::from_le_bytes([b[off + 2], b[off + 3]]));
                    (off + size <= b.len()).then(|| Some(b[off..off + size].to_vec()))
                }
            }
        };
        Some(Parts {
            revision: b[0],
            sbz1: b[1],
            control: c & !SE_SELF_RELATIVE,
            owner: sid_at(REL_OWNER)?,
            group: sid_at(REL_GROUP)?,
            sacl: acl_at(REL_SACL, c & SE_SACL_PRESENT != 0)?,
            dacl: acl_at(REL_DACL, c & SE_DACL_PRESENT != 0)?,
        })
    }
}

/// `MakeSelfRelativeSD` / `RtlMakeSelfRelativeSD` (`allow_relative`: the
/// native routine copies a descriptor that is already self-relative, the
/// others refuse it): write
/// into `dest` (if `*length` holds the need), else report the need in
/// `*length` with [`SecErr::BufferTooSmall`].
///
/// # Safety
/// `src` is readable; `dest` is writable for `*length` bytes if non-null.
pub unsafe fn make_self_relative(src: SdPtr, dest: *mut u8, length: &mut u32, allow_relative: bool) -> SecResult<()> {
    // SAFETY: as documented.
    unsafe {
        if src.is_relative() && !allow_relative {
            return Err(SecErr::BadDescriptorFormat);
        }
        let parts = Parts::read(src);
        let need = parts.relative_length();
        if dest.is_null() || (*length as usize) < need {
            *length = need as u32;
            return Err(SecErr::BufferTooSmall);
        }
        if src.is_relative() {
            core::ptr::copy_nonoverlapping(src.0, dest, need);
        } else {
            let bytes = parts.to_relative();
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len());
        }
    }
    Ok(())
}

/// One output buffer of [`make_absolute`].
#[derive(Debug)]
pub struct OutBuf<'a> {
    pub ptr: *mut u8,
    pub size: &'a mut u32,
}

/// `MakeAbsoluteSD` / `RtlSelfRelativeToAbsoluteSD`: split a self-relative
/// descriptor into an absolute header and copies of its parts. If any
/// buffer is too small, all five sizes are set to what is needed.
///
/// # Safety
/// `src` is readable; the buffers are writable for their sizes.
pub unsafe fn make_absolute(
    src: SdPtr,
    abs: OutBuf<'_>,
    dacl: OutBuf<'_>,
    sacl: OutBuf<'_>,
    owner: OutBuf<'_>,
    group: OutBuf<'_>,
) -> SecResult<()> {
    // SAFETY: as documented.
    unsafe {
        if !src.is_relative() {
            return Err(SecErr::BadDescriptorFormat);
        }
        let parts = Parts::read(src);
        let len = |p: &Option<Vec<u8>>| p.as_ref().map_or(0, Vec::len) as u32;
        let needs = [
            SECURITY_DESCRIPTOR_MIN_LENGTH as u32,
            len(&parts.dacl),
            len(&parts.sacl),
            len(&parts.owner),
            len(&parts.group),
        ];
        let bufs = [&abs, &dacl, &sacl, &owner, &group];
        let enough = bufs.iter().zip(needs).all(|(b, need)| need == 0 || (!b.ptr.is_null() && *b.size >= need));
        if !enough {
            for (b, need) in [abs, dacl, sacl, owner, group].into_iter().zip(needs) {
                *b.size = need;
            }
            return Err(SecErr::BufferTooSmall);
        }
        let put = |b: &OutBuf<'_>, data: &Option<Vec<u8>>| -> *mut u8 {
            match data {
                Some(d) => {
                    core::ptr::copy_nonoverlapping(d.as_ptr(), b.ptr, d.len());
                    b.ptr
                }
                None => core::ptr::null_mut(),
            }
        };
        let (dp, sp, op, gp) =
            (put(&dacl, &parts.dacl), put(&sacl, &parts.sacl), put(&owner, &parts.owner), put(&group, &parts.group));
        // The header: the source's first 20 bytes, then the control and the slots.
        core::ptr::copy_nonoverlapping(src.0, abs.ptr, RELATIVE_HEADER);
        let d = SdPtr(abs.ptr);
        d.set_control_word(parts.control);
        d.set_slot(ABS_OWNER, op);
        d.set_slot(ABS_GROUP, gp);
        d.set_slot(ABS_SACL, sp);
        d.set_slot(ABS_DACL, dp);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sid::{builtin, local_system, world};

    fn all_readable(p: *const u8, _n: usize) -> bool {
        !p.is_null()
    }

    fn sd_buf() -> Vec<u8> {
        alloc::vec![0xCCu8; 256]
    }

    #[test]
    fn create_and_set() {
        let mut b = sd_buf();
        let sd = SdPtr(b.as_mut_ptr());
        // SAFETY: the buffer is 256 bytes and the pointers outlive the calls.
        unsafe {
            assert_eq!(create(sd, 2), Err(SecErr::UnknownRevision));
            assert_eq!(b[0], 0xCC);
            create(sd, 1).unwrap();
            assert!(b[..40].iter().skip(1).all(|&x| x == 0) && b[0] == 1 && b[40] == 0xCC);
            assert!(is_valid(sd, &all_readable));
            assert_eq!(length(sd), 40);
            let mut owner = builtin(544).to_bytes();
            set_owner_or_group(sd, false, owner.as_mut_ptr(), true).unwrap();
            assert_eq!(sd.control(), SE_OWNER_DEFAULTED);
            assert_eq!(get_owner_or_group(sd, false), Ok((owner.as_mut_ptr(), true)));
            let mut group = local_system().to_bytes();
            set_owner_or_group(sd, true, group.as_mut_ptr(), false).unwrap();
            assert_eq!(length(sd), 40 + 16 + 12);
            // DACL: three states.
            assert_eq!(get_acl(sd, false).unwrap().present, false);
            set_acl(sd, false, true, core::ptr::null_mut(), false).unwrap();
            assert_eq!(sd.control(), SE_OWNER_DEFAULTED | SE_DACL_PRESENT);
            let mut dacl = acl::build(2, &[(0, 0, 0x1F01FF, &world())]);
            set_acl(sd, false, true, dacl.as_mut_ptr(), true).unwrap();
            assert_eq!(sd.control(), SE_OWNER_DEFAULTED | SE_DACL_PRESENT | SE_DACL_DEFAULTED);
            set_acl(sd, false, false, core::ptr::null_mut(), false).unwrap();
            assert_eq!(sd.control(), SE_OWNER_DEFAULTED | SE_DACL_DEFAULTED);
            assert!(is_valid(sd, &all_readable));
            // Control bits.
            assert_eq!(set_control(sd, SE_DACL_PROTECTED, SE_DACL_PROTECTED), Ok(()));
            assert_eq!(set_control(sd, SE_DACL_PRESENT, 0), Err(SecErr::InvalidParameter));
            assert_eq!(set_control(sd, 0, 0), Err(SecErr::InvalidParameter));
            assert_eq!(set_control(sd, SE_DACL_PROTECTED, SE_SACL_PROTECTED), Err(SecErr::InvalidParameter));
        }
    }

    #[test]
    fn relative_round_trip() {
        let mut b = sd_buf();
        let sd = SdPtr(b.as_mut_ptr());
        let mut owner = builtin(544).to_bytes();
        let mut group = local_system().to_bytes();
        let mut dacl = acl::build(2, &[(0, 0, 0x1000_0000, &world())]);
        let mut sacl = acl::build(2, &[(2, 0xC0, 0x1000_0000, &world())]);
        // SAFETY: as above.
        unsafe {
            create(sd, 1).unwrap();
            set_owner_or_group(sd, false, owner.as_mut_ptr(), false).unwrap();
            set_owner_or_group(sd, true, group.as_mut_ptr(), false).unwrap();
            set_acl(sd, false, true, dacl.as_mut_ptr(), false).unwrap();
            set_acl(sd, true, true, sacl.as_mut_ptr(), false).unwrap();
            assert_eq!(length(sd), 40 + 16 + 12 + 28 + 28);
            let mut need = 0u32;
            assert_eq!(make_self_relative(sd, core::ptr::null_mut(), &mut need, false), Err(SecErr::BufferTooSmall));
            assert_eq!(need, 20 + 16 + 12 + 28 + 28);
            let mut rel = alloc::vec![0u8; need as usize];
            make_self_relative(sd, rel.as_mut_ptr(), &mut need, false).unwrap();
            // SACL, DACL, owner, group.
            assert_eq!(u32::from_le_bytes(rel[12..16].try_into().unwrap()), 20);
            assert_eq!(u32::from_le_bytes(rel[16..20].try_into().unwrap()), 48);
            assert_eq!(u32::from_le_bytes(rel[4..8].try_into().unwrap()), 76);
            assert_eq!(u32::from_le_bytes(rel[8..12].try_into().unwrap()), 92);
            let rsd = SdPtr(rel.as_mut_ptr());
            assert!(is_valid(rsd, &all_readable));
            assert_eq!(length(rsd), rel.len());
            assert_eq!(get_owner_or_group(rsd, false).unwrap().0 as usize, rel.as_ptr() as usize + 76);
            assert_eq!(get_acl(rsd, false).unwrap().acl as usize, rel.as_ptr() as usize + 48);
            assert_eq!(
                make_self_relative(rsd, core::ptr::null_mut(), &mut need, false),
                Err(SecErr::BadDescriptorFormat)
            );
            // Back to absolute.
            let (mut a, mut d, mut s, mut o, mut g) = (
                alloc::vec![0xAAu8; 64],
                alloc::vec![0xAAu8; 64],
                alloc::vec![0xAAu8; 64],
                alloc::vec![0xAAu8; 64],
                alloc::vec![0xAAu8; 64],
            );
            let (mut sa, mut sd_, mut ss, mut so, mut sg) = (64u32, 64u32, 64u32, 64u32, 64u32);
            make_absolute(
                rsd,
                OutBuf { ptr: a.as_mut_ptr(), size: &mut sa },
                OutBuf { ptr: d.as_mut_ptr(), size: &mut sd_ },
                OutBuf { ptr: s.as_mut_ptr(), size: &mut ss },
                OutBuf { ptr: o.as_mut_ptr(), size: &mut so },
                OutBuf { ptr: g.as_mut_ptr(), size: &mut sg },
            )
            .unwrap();
            let asd = SdPtr(a.as_mut_ptr());
            assert!(is_valid(asd, &all_readable));
            assert_eq!(asd.dacl_slot(), d.as_mut_ptr());
            assert_eq!(&d[..28], &dacl[..]);
            assert_eq!(d[28], 0xAA);
            assert_eq!(asd.control(), SE_DACL_PRESENT | SE_SACL_PRESENT);
            // Too small: every size is set.
            let (mut sa, mut sd_, mut ss, mut so, mut sg) = (40u32, 28u32, 28u32, 16u32, 11u32);
            let e = make_absolute(
                rsd,
                OutBuf { ptr: a.as_mut_ptr(), size: &mut sa },
                OutBuf { ptr: d.as_mut_ptr(), size: &mut sd_ },
                OutBuf { ptr: s.as_mut_ptr(), size: &mut ss },
                OutBuf { ptr: o.as_mut_ptr(), size: &mut so },
                OutBuf { ptr: g.as_mut_ptr(), size: &mut sg },
            );
            assert_eq!(e, Err(SecErr::BufferTooSmall));
            assert_eq!((sa, sd_, ss, so, sg), (40, 28, 28, 16, 12));
        }
    }

    #[test]
    fn invalid_descriptors() {
        let mut b = sd_buf();
        let sd = SdPtr(b.as_mut_ptr());
        // SAFETY: as above.
        unsafe {
            create(sd, 1).unwrap();
            b[0] = 2;
            assert!(!is_valid(sd, &all_readable));
            assert_eq!(get_acl(sd, false), Err(SecErr::UnknownRevision));
            assert_eq!(check_absolute(sd), Err(SecErr::UnknownRevision));
            b[0] = 1;
            let mut bad = [2u8, 0, 0, 0, 0, 0, 0, 5];
            set_owner_or_group(sd, false, bad.as_mut_ptr(), false).unwrap();
            assert!(!is_valid(sd, &all_readable));
            // A relative one that clears its own flag is not an absolute one.
            let rel = Parts { revision: 1, ..Parts::default() }.to_relative();
            let mut r = rel.clone();
            r[3] = 0;
            r.resize(64, 0);
            assert!(!is_valid(SdPtr(r.as_mut_ptr()), &all_readable) || r[4..20].iter().all(|&x| x == 0));
        }
    }
}
