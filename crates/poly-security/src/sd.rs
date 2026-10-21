//! Security descriptors (`winnt.h`).
//!
//! The absolute form, `SECURITY_DESCRIPTOR`, on x64: `Revision` (byte),
//! `Sbz1`, `Control` (word), then four pointers at 8, 16, 24 and 32 —
//! `Owner`, `Group`, `Sacl`, `Dacl` — 40 bytes
//! (`SECURITY_DESCRIPTOR_MIN_LENGTH`). The self-relative form,
//! `SECURITY_DESCRIPTOR_RELATIVE`, has `SE_SELF_RELATIVE` in its control
//! and four `DWORD` offsets from its start at 4, 8, 12 and 16 (0: absent),
//! 20 bytes, followed by the parts.
//!
//! The DLL reads and writes the absolute form's pointers itself; this module
//! holds the control-bit rules and the self-relative form.
//!
//! Choices to be confirmed by the spec agent: `MakeSelfRelativeSD` lays the
//! parts out as owner, group, SACL, DACL, each at a 4-byte boundary;
//! `GetSecurityDescriptorLength` of an absolute descriptor is 40 plus its
//! parts' lengths; a present DACL with a null pointer (a NULL DACL) keeps
//! `SE_DACL_PRESENT` with offset 0 in the self-relative form.

use crate::acl;
use crate::error::*;
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
pub const SE_DACL_AUTO_INHERIT_REQ: u16 = 0x0100;
pub const SE_SACL_AUTO_INHERIT_REQ: u16 = 0x0200;
pub const SE_DACL_AUTO_INHERITED: u16 = 0x0400;
pub const SE_SACL_AUTO_INHERITED: u16 = 0x0800;
pub const SE_DACL_PROTECTED: u16 = 0x1000;
pub const SE_SACL_PROTECTED: u16 = 0x2000;
pub const SE_RM_CONTROL_VALID: u16 = 0x4000;
pub const SE_SELF_RELATIVE: u16 = 0x8000;

/// The bits `SetSecurityDescriptorControl` may change (Learn: "the
/// following flags can be set": the auto-inherit and protected bits).
pub const SETTABLE_CONTROL: u16 = SE_DACL_AUTO_INHERIT_REQ
    | SE_SACL_AUTO_INHERIT_REQ
    | SE_DACL_AUTO_INHERITED
    | SE_SACL_AUTO_INHERITED
    | SE_DACL_PROTECTED
    | SE_SACL_PROTECTED;

/// The control after `SetSecurityDescriptorDacl` (or `...Sacl` with the
/// SACL bits): present sets the present bit and the defaulted bit from
/// `defaulted`; absent clears both.
pub fn set_acl_control(control: u16, present_bit: u16, defaulted_bit: u16, present: bool, defaulted: bool) -> u16 {
    let c = control & !(present_bit | defaulted_bit);
    if !present {
        c
    } else if defaulted {
        c | present_bit | defaulted_bit
    } else {
        c | present_bit
    }
}

/// The checks every `Set*` function makes on an absolute descriptor's
/// header: revision 1 (`ERROR_UNKNOWN_REVISION`) and not self-relative
/// (`ERROR_INVALID_SECURITY_DESCR`).
pub fn check_absolute(revision: u8, control: u16) -> Result<(), u32> {
    if revision != SECURITY_DESCRIPTOR_REVISION {
        Err(ERROR_UNKNOWN_REVISION)
    } else if control & SE_SELF_RELATIVE != 0 {
        Err(ERROR_INVALID_SECURITY_DESCR)
    } else {
        Ok(())
    }
}

/// `SetSecurityDescriptorControl`: change the `mask` bits to `bits`.
pub fn set_control(control: u16, mask: u16, bits: u16) -> Result<u16, u32> {
    if mask & !SETTABLE_CONTROL != 0 || bits & !mask != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok((control & !mask) | bits)
}

/// The four parts of a descriptor as byte slices (each exactly its length:
/// a SID's or an ACL's `AclSize`), and the control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Parts<'a> {
    pub control: u16,
    pub owner: Option<&'a [u8]>,
    pub group: Option<&'a [u8]>,
    pub sacl: Option<&'a [u8]>,
    pub dacl: Option<&'a [u8]>,
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

impl Parts<'_> {
    /// The self-relative form's length.
    pub fn relative_length(&self) -> usize {
        let mut n = RELATIVE_HEADER;
        for p in [self.owner, self.group, self.sacl, self.dacl].into_iter().flatten() {
            n = align4(n) + p.len();
        }
        n
    }

    /// The absolute form's length (40 plus each part).
    pub fn absolute_length(&self) -> usize {
        SECURITY_DESCRIPTOR_MIN_LENGTH
            + [self.owner, self.group, self.sacl, self.dacl].into_iter().flatten().map(<[u8]>::len).sum::<usize>()
    }

    /// The self-relative form (`MakeSelfRelativeSD`).
    pub fn to_relative(&self) -> Vec<u8> {
        let mut v = alloc::vec![0u8; RELATIVE_HEADER];
        v[0] = SECURITY_DESCRIPTOR_REVISION;
        let control = self.control | SE_SELF_RELATIVE;
        v[2..4].copy_from_slice(&control.to_le_bytes());
        for (at, part) in
            [(REL_OWNER, self.owner), (REL_GROUP, self.group), (REL_SACL, self.sacl), (REL_DACL, self.dacl)]
        {
            if let Some(p) = part {
                v.resize(align4(v.len()), 0);
                let off = v.len() as u32;
                v[at..at + 4].copy_from_slice(&off.to_le_bytes());
                v.extend_from_slice(p);
            }
        }
        v
    }

    /// Check each part (`IsValidSecurityDescriptor`): valid SIDs and ACLs.
    pub fn check(&self) -> Result<(), u32> {
        for s in [self.owner, self.group].into_iter().flatten() {
            if !sid::is_valid(s) {
                return Err(ERROR_INVALID_SECURITY_DESCR);
            }
        }
        for a in [self.sacl, self.dacl].into_iter().flatten() {
            if !acl::is_valid(a) {
                return Err(ERROR_INVALID_SECURITY_DESCR);
            }
        }
        Ok(())
    }
}

fn dword(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

/// The parts of a self-relative descriptor `b` (as long as the caller
/// could tell: the offsets must stay inside it).
pub fn relative_parts(b: &[u8]) -> Result<Parts<'_>, u32> {
    if b.len() < RELATIVE_HEADER {
        return Err(ERROR_INVALID_SECURITY_DESCR);
    }
    if b[0] != SECURITY_DESCRIPTOR_REVISION {
        return Err(ERROR_UNKNOWN_REVISION);
    }
    let control = u16::from_le_bytes([b[2], b[3]]);
    if control & SE_SELF_RELATIVE == 0 {
        return Err(ERROR_BAD_DESCRIPTOR_FORMAT);
    }
    let sid_at = |at: usize| -> Result<Option<&[u8]>, u32> {
        match dword(b, at) {
            0 => Ok(None),
            off if off < RELATIVE_HEADER || off >= b.len() => Err(ERROR_INVALID_SECURITY_DESCR),
            off => sid::valid_length(&b[off..]).map(|l| Some(&b[off..off + l])).ok_or(ERROR_INVALID_SECURITY_DESCR),
        }
    };
    let acl_at = |at: usize, present: bool| -> Result<Option<&[u8]>, u32> {
        match dword(b, at) {
            0 => Ok(None),
            _ if !present => Ok(None),
            off if off < RELATIVE_HEADER || off + acl::ACL_HEADER > b.len() => Err(ERROR_INVALID_SECURITY_DESCR),
            off => {
                let size = usize::from(u16::from_le_bytes([b[off + 2], b[off + 3]]));
                if off + size > b.len() { Err(ERROR_INVALID_SECURITY_DESCR) } else { Ok(Some(&b[off..off + size])) }
            }
        }
    };
    Ok(Parts {
        control,
        owner: sid_at(REL_OWNER)?,
        group: sid_at(REL_GROUP)?,
        sacl: acl_at(REL_SACL, control & SE_SACL_PRESENT != 0)?,
        dacl: acl_at(REL_DACL, control & SE_DACL_PRESENT != 0)?,
    })
}

/// The length of a self-relative descriptor: the end of its last part.
pub fn relative_length(b: &[u8]) -> Result<usize, u32> {
    let p = relative_parts(b)?;
    let end = |part: Option<&[u8]>| part.map_or(0, |s| s.as_ptr() as usize - b.as_ptr() as usize + s.len());
    Ok([RELATIVE_HEADER, end(p.owner), end(p.group), end(p.sacl), end(p.dacl)]
        .into_iter()
        .max()
        .unwrap_or(RELATIVE_HEADER))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sid::{builtin, world};

    #[test]
    fn dacl_control_rules() {
        assert_eq!(set_acl_control(0, SE_DACL_PRESENT, SE_DACL_DEFAULTED, true, false), SE_DACL_PRESENT);
        assert_eq!(
            set_acl_control(0, SE_DACL_PRESENT, SE_DACL_DEFAULTED, true, true),
            SE_DACL_PRESENT | SE_DACL_DEFAULTED
        );
        assert_eq!(
            set_acl_control(SE_DACL_PRESENT | SE_DACL_DEFAULTED | 1, SE_DACL_PRESENT, SE_DACL_DEFAULTED, false, true),
            1
        );
        assert_eq!(check_absolute(1, 0), Ok(()));
        assert_eq!(check_absolute(2, 0), Err(ERROR_UNKNOWN_REVISION));
        assert_eq!(check_absolute(1, SE_SELF_RELATIVE), Err(ERROR_INVALID_SECURITY_DESCR));
        assert_eq!(
            set_control(SE_DACL_PRESENT, SE_DACL_PROTECTED, SE_DACL_PROTECTED),
            Ok(SE_DACL_PRESENT | SE_DACL_PROTECTED)
        );
        assert_eq!(set_control(0, SE_DACL_PRESENT, SE_DACL_PRESENT), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set_control(0, SE_DACL_PROTECTED, SE_SACL_PROTECTED), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn self_relative_round_trip() {
        let owner = builtin(544).to_bytes();
        let dacl = acl::build(2, &[(0, 0, 0x1F_01FF, &world())]);
        let parts = Parts { control: SE_DACL_PRESENT, owner: Some(&owner), group: None, sacl: None, dacl: Some(&dacl) };
        assert!(parts.check().is_ok());
        let rel = parts.to_relative();
        assert_eq!(rel.len(), parts.relative_length());
        assert_eq!(rel.len(), 20 + 16 + dacl.len());
        assert_eq!(&rel[..4], &[1, 0, 0x04, 0x80]);
        assert_eq!(dword(&rel, REL_OWNER), 20);
        assert_eq!(dword(&rel, REL_GROUP), 0);
        assert_eq!(dword(&rel, REL_DACL), 36);
        let back = relative_parts(&rel).unwrap();
        assert_eq!(back.owner, Some(&owner[..]));
        assert_eq!(back.dacl, Some(&dacl[..]));
        assert_eq!(back.group, None);
        assert_eq!(back.control, SE_DACL_PRESENT | SE_SELF_RELATIVE);
        assert_eq!(relative_length(&rel), Ok(rel.len()));
        assert_eq!(parts.absolute_length(), 40 + 16 + dacl.len());
    }

    #[test]
    fn a_null_dacl_stays_present() {
        let parts = Parts { control: SE_DACL_PRESENT, ..Parts::default() };
        let rel = parts.to_relative();
        assert_eq!(rel.len(), 20);
        let back = relative_parts(&rel).unwrap();
        assert_eq!(back.dacl, None);
        assert_ne!(back.control & SE_DACL_PRESENT, 0);
    }

    #[test]
    fn bad_descriptors() {
        assert_eq!(relative_parts(&[1, 0, 0, 0]), Err(ERROR_INVALID_SECURITY_DESCR));
        let mut rel = Parts { control: 0, owner: None, ..Parts::default() }.to_relative();
        rel[0] = 3;
        assert_eq!(relative_parts(&rel), Err(ERROR_UNKNOWN_REVISION));
        rel[0] = 1;
        rel[3] = 0;
        assert_eq!(relative_parts(&rel), Err(ERROR_BAD_DESCRIPTOR_FORMAT));
        rel[3] = 0x80;
        rel[REL_OWNER] = 200;
        assert_eq!(relative_parts(&rel), Err(ERROR_INVALID_SECURITY_DESCR));
        let bad_sid = [2u8, 0, 0, 0, 0, 0, 0, 0];
        let p = Parts { owner: Some(&bad_sid), ..Parts::default() };
        assert_eq!(p.check(), Err(ERROR_INVALID_SECURITY_DESCR));
    }
}
