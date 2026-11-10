//! Security descriptor strings (SDDL, `sddl.h`; Learn: "Security Descriptor
//! String Format", "ACE Strings", "SID Strings"):
//! `ConvertStringSecurityDescriptorToSecurityDescriptorW` and
//! `ConvertSecurityDescriptorToStringSecurityDescriptorW`.
//!
//! What the parser accepts and refuses, and which error each failure gets,
//! is what the spec sheets observed (the codes are a little irregular:
//! 87 for a malformed structure, 1332 for an empty SID, 1337 for a SID that
//! is not one, 1336 for rights that are not rights, 1004 for flags,
//! 1804 for an ACE type that is unknown or in the wrong section, 1705 for a
//! GUID). Conditional ACEs (`XA`, `XD`, `XU`), resource attributes (`RA`)
//! and scoped policy IDs (`SP`) carry their data as [`condace`] builds it.

use crate::access::GENERIC_ALL;
use crate::acl;
use crate::condace;
use crate::err::{SecErr, SecResult};
use crate::sd::{self, Parts};
use crate::sddl;
use crate::sid::{self, Sid};
use alloc::string::String;
use alloc::vec::Vec;

fn is_space(c: u16) -> bool {
    c == 0x20
}

/// Rights strings (Learn: "ACE Strings"): generic, standard, file, key and
/// object-specific rights.
const RIGHTS: &[(&str, u32)] = &[
    ("GA", 0x1000_0000),
    ("GR", 0x8000_0000),
    ("GW", 0x4000_0000),
    ("GX", 0x2000_0000),
    ("FA", 0x001f_01ff),
    ("FR", 0x0012_0089),
    ("FW", 0x0012_0116),
    ("FX", 0x0012_00a0),
    ("KA", 0x000f_003f),
    ("KR", 0x0002_0019),
    ("KW", 0x0002_0006),
    ("KX", 0x0002_0019),
    ("RC", 0x0002_0000),
    ("SD", 0x0001_0000),
    ("WD", 0x0004_0000),
    ("WO", 0x0008_0000),
    ("CC", 0x1),
    ("DC", 0x2),
    ("LC", 0x4),
    ("SW", 0x8),
    ("RP", 0x10),
    ("WP", 0x20),
    ("DT", 0x40),
    ("LO", 0x80),
    ("CR", 0x100),
];

/// The aliases that name a single right, in the order they are written:
/// generic, then the object-specific ones, then the standard ones.
const WRITE_ORDER: &[(&str, u32)] = &[
    ("GA", 0x1000_0000),
    ("GX", 0x2000_0000),
    ("GW", 0x4000_0000),
    ("GR", 0x8000_0000),
    ("CC", 0x1),
    ("DC", 0x2),
    ("LC", 0x4),
    ("SW", 0x8),
    ("RP", 0x10),
    ("WP", 0x20),
    ("DT", 0x40),
    ("LO", 0x80),
    ("CR", 0x100),
    ("SD", 0x0001_0000),
    ("RC", 0x0002_0000),
    ("WD", 0x0004_0000),
    ("WO", 0x0008_0000),
];

/// The composite aliases printed when the mask equals them.
const COMPOSITES: &[(&str, u32)] = &[
    ("FA", 0x001f_01ff),
    ("FR", 0x0012_0089),
    ("FW", 0x0012_0116),
    ("FX", 0x0012_00a0),
    ("KA", 0x000f_003f),
    ("KR", 0x0002_0019),
    ("KW", 0x0002_0006),
];

const LABEL_RIGHTS: &[(&str, u32)] = &[("NW", 1), ("NR", 2), ("NX", 4)];

fn alias(table: &[(&str, u32)], a: u16, b: u16) -> Option<u32> {
    let (a, b) = (u8::try_from(a).ok()?.to_ascii_uppercase(), u8::try_from(b).ok()?.to_ascii_uppercase());
    table.iter().find(|(n, _)| n.as_bytes() == [a, b]).map(|&(_, v)| v)
}

/// A rights field: aliases, `0x` hexadecimal (saturating), decimal with an
/// optional minus, concatenated and OR-ed; a leading space is skipped,
/// anything else unknown is an error.
fn parse_rights(s: &[u16], table: &[(&str, u32)]) -> SecResult<u32> {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut mask = 0u32;
    while i < s.len() {
        let c = s[i];
        if c == u16::from(b'0') && s.get(i + 1).is_some_and(|x| (*x | 0x20) == u16::from(b'x')) {
            i += 2;
            let start = i;
            let mut v: u64 = 0;
            while i < s.len() && char::from_u32(u32::from(s[i])).is_some_and(|ch| ch.is_ascii_hexdigit()) {
                v = (v << 4 | u64::from(char::from_u32(u32::from(s[i])).unwrap().to_digit(16).unwrap()))
                    .min(u64::from(u32::MAX));
                i += 1;
            }
            if i == start {
                return Err(SecErr::InvalidAcl);
            }
            mask |= v as u32;
        } else if (u16::from(b'0')..=u16::from(b'9')).contains(&c) || c == u16::from(b'-') {
            let neg = c == u16::from(b'-');
            if neg {
                i += 1;
            }
            let start = i;
            let mut v: u64 = 0;
            while i < s.len() && (u16::from(b'0')..=u16::from(b'9')).contains(&s[i]) {
                v = (v * 10 + u64::from(s[i] - u16::from(b'0'))).min(u64::from(u32::MAX));
                i += 1;
            }
            if i == start {
                return Err(SecErr::InvalidAcl);
            }
            mask |= if neg { (v as u32).wrapping_neg() } else { v as u32 };
        } else {
            let Some(&d) = s.get(i + 1) else { return Err(SecErr::InvalidAcl) };
            mask |= alias(table, c, d).ok_or(SecErr::InvalidAcl)?;
            i += 2;
        }
    }
    Ok(mask)
}

/// A SID token at the start of `s`: the consumed length and the SID bytes.
/// `Err(NoneMapped)` for nothing at all.
fn sid_token(s: &[u16]) -> SecResult<(usize, Vec<u8>)> {
    if s.is_empty() {
        return Err(SecErr::NoneMapped);
    }
    let upper = |c: u16| u8::try_from(c).map(|b| b.to_ascii_uppercase()).unwrap_or(0);
    if upper(s[0]) == b'S' && s.get(1) == Some(&u16::from(b'-')) {
        // The SID's text ends at a space or where the next part (a letter
        // and a colon) starts: a hexadecimal number may end in `D`, so the
        // digits alone do not say. Whatever else is in between is the
        // SID's and makes it a bad one ("S-1-5-18x").
        let n = (2..s.len()).find(|&i| is_space(s[i]) || s.get(i + 1) == Some(&u16::from(b':'))).unwrap_or(s.len());
        let bytes = sddl::parse_standard(&s[..n]).ok_or(SecErr::InvalidSid)?;
        return Ok((n, bytes));
    }
    if s.len() < 2 {
        return Err(SecErr::InvalidSid);
    }
    let (a, b) =
        (u8::try_from(s[0]).map_err(|_| SecErr::InvalidSid)?, u8::try_from(s[1]).map_err(|_| SecErr::InvalidSid)?);
    let sid = sddl::alias_sid(a, b).ok_or(SecErr::InvalidSid)?;
    Ok((2, sid.to_bytes()))
}

fn guid_bytes(s: &[u16]) -> SecResult<[u8; 16]> {
    // 8-4-4-4-12 hexadecimal digits, no braces.
    let t: Vec<char> = s.iter().filter_map(|&c| char::from_u32(u32::from(c))).collect();
    if t.len() != 36 || s.len() != 36 {
        return Err(SecErr::InvalidUuid);
    }
    let mut hex = Vec::new();
    for (i, ch) in t.iter().enumerate() {
        if [8, 13, 18, 23].contains(&i) {
            if *ch != '-' {
                return Err(SecErr::InvalidUuid);
            }
        } else {
            hex.push(ch.to_digit(16).ok_or(SecErr::InvalidUuid)? as u8);
        }
    }
    let byte = |i: usize| hex[2 * i] << 4 | hex[2 * i + 1];
    let b: Vec<u8> = (0..16).map(byte).collect();
    // Data1 (4, little endian), Data2, Data3 (2 each), Data4 (8 as written).
    Ok([b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]])
}

/// An ACE string `type;flags;rights;object;inherited;sid` as the ACE's
/// bytes (and whether it needs a revision 4 ACL).
fn parse_ace(f: &[&[u16]], sacl: bool) -> SecResult<(Vec<u8>, bool, usize)> {
    if f.len() != 6 && f.len() != 7 {
        return Err(SecErr::InvalidParameter);
    }
    let text: String =
        f[0].iter().filter_map(|&c| char::from_u32(u32::from(c))).collect::<String>().trim().to_ascii_uppercase();
    // A seventh field is the condition of a conditional ACE or the
    // attribute of a resource attribute ACE; no other type has one.
    let conditional = matches!(text.as_str(), "XA" | "XD" | "XU");
    let attribute = text == "RA";
    if f.len() == 7 && !(conditional || attribute || text == "ZA") {
        return Err(SecErr::InvalidParameter);
    }
    // (type, object type?, audit?)
    let (kind, object, audit) = match text.as_str() {
        "XA" if !sacl => (acl::ACCESS_ALLOWED_CALLBACK_ACE_TYPE, false, false),
        "XD" if !sacl => (acl::ACCESS_DENIED_CALLBACK_ACE_TYPE, false, false),
        "XA" | "XD" => return Err(SecErr::InvalidDatatype),
        "XU" if sacl => (acl::SYSTEM_AUDIT_CALLBACK_ACE_TYPE, false, true),
        "XU" => return Err(SecErr::InvalidAcl),
        "RA" if sacl => (acl::SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE, false, false),
        "SP" if sacl => (acl::SYSTEM_SCOPED_POLICY_ID_ACE_TYPE, false, false),
        "RA" | "SP" => return Err(SecErr::InvalidDatatype),
        "A" if !sacl => (acl::ACCESS_ALLOWED_ACE_TYPE, false, false),
        "D" if !sacl => (acl::ACCESS_DENIED_ACE_TYPE, false, false),
        "A" | "D" | "OA" | "OD" if sacl => return Err(SecErr::InvalidDatatype),
        "OA" => (acl::ACCESS_ALLOWED_OBJECT_ACE_TYPE, true, false),
        "OD" => (acl::ACCESS_DENIED_OBJECT_ACE_TYPE, true, false),
        "AU" if sacl => (acl::SYSTEM_AUDIT_ACE_TYPE, false, true),
        "AL" => return Err(SecErr::InvalidParameter),
        "OU" if sacl => (acl::SYSTEM_AUDIT_OBJECT_ACE_TYPE, true, true),
        "OL" if sacl => (acl::SYSTEM_ALARM_OBJECT_ACE_TYPE, true, true),
        "ML" if sacl => (acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE, false, false),
        "ML" => return Err(SecErr::InvalidDatatype),
        "AU" | "OU" | "OL" => return Err(SecErr::InvalidAcl),
        "ZA" | "AA" => return Err(SecErr::InvalidAcl),
        "TL" => return Err(SecErr::InvalidParameter),
        _ => return Err(SecErr::InvalidDatatype),
    };
    // Flags.
    let mut flags = 0u8;
    let fl: Vec<u16> = f[1].iter().copied().filter(|&c| !is_space(c)).collect();
    if !fl.len().is_multiple_of(2) {
        return Err(SecErr::InvalidFlags);
    }
    for p in fl.chunks(2) {
        let (a, b) = (
            u8::try_from(p[0]).unwrap_or(0).to_ascii_uppercase(),
            u8::try_from(p[1]).unwrap_or(0).to_ascii_uppercase(),
        );
        flags |= match [a, b] {
            [b'C', b'I'] => acl::CONTAINER_INHERIT_ACE,
            [b'O', b'I'] => acl::OBJECT_INHERIT_ACE,
            [b'N', b'P'] => acl::NO_PROPAGATE_INHERIT_ACE,
            [b'I', b'O'] => acl::INHERIT_ONLY_ACE,
            [b'I', b'D'] => acl::INHERITED_ACE,
            [b'S', b'A'] if audit => acl::SUCCESSFUL_ACCESS_ACE_FLAG,
            [b'F', b'A'] if audit => acl::FAILED_ACCESS_ACE_FLAG,
            _ => return Err(SecErr::InvalidFlags),
        };
    }
    let label = kind == acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE;
    let mask = parse_rights(f[2], if label { LABEL_RIGHTS } else { RIGHTS })?;
    // The SID: the rest of its field, a trailing space allowed.
    let sid_field = {
        let mut s = f[5];
        while let [first, rest @ ..] = s {
            if is_space(*first) { s = rest } else { break }
        }
        s
    };
    let (n, sid_bytes) =
        sid_token(sid_field).map_err(|e| if e == SecErr::NoneMapped { SecErr::InvalidSid } else { e })?;
    if sid_field[n..].iter().any(|&c| !is_space(c)) {
        return Err(SecErr::InvalidSid);
    }
    if label {
        let ok = sid::valid_length(&sid_bytes).is_some()
            && sid::authority_of(&sid_bytes[2..8]) == sid::SECURITY_MANDATORY_LABEL_AUTHORITY;
        if !ok {
            return Err(SecErr::InvalidParameter);
        }
    }
    let (g1, g2) = (f[3], f[4]);
    let mut out = Vec::new();
    let has_guids = !g1.is_empty() || !g2.is_empty();
    // The data after the SID: the seventh field between its parentheses.
    let mut data = Vec::new();
    if let Some(field) = f.get(6) {
        let inner = match trim(field) {
            [open, inner @ .., close] if *open == u16::from(b'(') && *close == u16::from(b')') => inner,
            _ => return Err(SecErr::InvalidAcl),
        };
        data = if attribute { condace::compile_attribute(inner)? } else { condace::compile(inner)? };
    } else if attribute {
        return Err(SecErr::InvalidParameter);
    }
    if object && has_guids {
        let mut oflags = 0u32;
        let mut guids = Vec::new();
        if !g1.is_empty() {
            oflags |= 1;
            guids.extend_from_slice(&guid_bytes(g1)?);
        }
        if !g2.is_empty() {
            oflags |= 2;
            guids.extend_from_slice(&guid_bytes(g2)?);
        }
        let size = acl::ACE_FIXED + 4 + guids.len() + sid_bytes.len();
        out.extend_from_slice(&[kind, flags]);
        out.extend_from_slice(&(size as u16).to_le_bytes());
        out.extend_from_slice(&mask.to_le_bytes());
        out.extend_from_slice(&oflags.to_le_bytes());
        out.extend_from_slice(&guids);
        out.extend_from_slice(&sid_bytes);
        return Ok((out, true, 0));
    }
    if !object && has_guids {
        return Err(SecErr::InvalidAcl);
    }
    // A plain ACE: an object type without GUIDs is the plain type.
    let plain = match kind {
        acl::ACCESS_ALLOWED_OBJECT_ACE_TYPE => acl::ACCESS_ALLOWED_ACE_TYPE,
        acl::ACCESS_DENIED_OBJECT_ACE_TYPE => acl::ACCESS_DENIED_ACE_TYPE,
        acl::SYSTEM_AUDIT_OBJECT_ACE_TYPE => acl::SYSTEM_AUDIT_ACE_TYPE,
        acl::SYSTEM_ALARM_OBJECT_ACE_TYPE => acl::SYSTEM_ALARM_ACE_TYPE,
        k => k,
    };
    let size = acl::ACE_FIXED + sid_bytes.len() + data.len();
    out.extend_from_slice(&[plain, flags]);
    out.extend_from_slice(&(size as u16).to_le_bytes());
    out.extend_from_slice(&mask.to_le_bytes());
    out.extend_from_slice(&sid_bytes);
    out.extend_from_slice(&data);
    // An object type without GUIDs is written as the plain type, but the
    // list is sized for the object form: four bytes the ACE does not use.
    Ok((out, object, if object { 4 } else { 0 }))
}

/// Split an ACE's text (between the parentheses) at `;`: six fields and,
/// if there is more, a seventh that is all the rest (a condition may hold
/// `;` in its strings).
fn fields(s: &[u16]) -> Vec<&[u16]> {
    s.splitn(7, |&c| c == u16::from(b';')).collect()
}

/// `s` without the spaces around it.
fn trim(mut s: &[u16]) -> &[u16] {
    while let [first, rest @ ..] = s {
        if is_space(*first) { s = rest } else { break }
    }
    while let [rest @ .., last] = s {
        if is_space(*last) { s = rest } else { break }
    }
    s
}

/// The position of the `)` that closes the ACE whose text starts at
/// `start`: the first one, except that a seventh field (after six `;`) is a
/// parenthesized expression, whose own parentheses (outside its quoted
/// strings) are skipped as a whole.
fn ace_end(s: &[u16], start: usize) -> Option<usize> {
    let is = |i: usize, c: u8| s[i] == u16::from(c);
    let mut semicolons = 0;
    let mut i = start;
    while i < s.len() {
        if is(i, b';') {
            semicolons += 1;
        } else if is(i, b'(') && semicolons >= 6 {
            let (mut depth, mut quoted) = (0usize, false);
            loop {
                if i >= s.len() {
                    return None;
                }
                if is(i, b'"') {
                    quoted = !quoted;
                } else if !quoted && is(i, b'(') {
                    depth += 1;
                } else if !quoted && is(i, b')') {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                i += 1;
            }
        } else if is(i, b')') {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// An ACL section's flags and entries.
struct Section {
    /// Whether an ACE (or a lone `(`) was read: text after it is ignored.
    entries: bool,
    control: u16,
    /// `None`: a NULL ACL.
    acl: Option<Vec<u8>>,
}

/// Parse the part after `D:` or `S:` (up to the next section), returning
/// what it holds and the position after the ACEs. Anything after the last
/// ACE that is not a section is ignored by the caller.
fn parse_acl_section(s: &[u16], sacl: bool) -> SecResult<(Section, usize)> {
    let (p_bit, ar_bit, ai_bit) = if sacl {
        (sd::SE_SACL_PROTECTED, sd::SE_SACL_AUTO_INHERIT_REQ, sd::SE_SACL_AUTO_INHERITED)
    } else {
        (sd::SE_DACL_PROTECTED, sd::SE_DACL_AUTO_INHERIT_REQ, sd::SE_DACL_AUTO_INHERITED)
    };
    let present = if sacl { sd::SE_SACL_PRESENT } else { sd::SE_DACL_PRESENT };
    let up = |c: u16| u8::try_from(c).map(|b| b.to_ascii_uppercase()).unwrap_or(0);
    let mut control = present;
    let mut i = 0;
    while i < s.len() {
        match up(s[i]) {
            b'P' => {
                control |= p_bit;
                i += 1;
            }
            b'A' if s.get(i + 1).is_some_and(|&c| up(c) == b'R') => {
                control |= ar_bit;
                i += 2;
            }
            b'A' if s.get(i + 1).is_some_and(|&c| up(c) == b'I') => {
                control |= ai_bit;
                i += 2;
            }
            _ => break,
        }
    }
    // NO_ACCESS_CONTROL: a NULL ACL, nothing after it but the next section.
    let rest = &s[i..];
    let no_ac: Vec<u16> = "NO_ACCESS_CONTROL".encode_utf16().collect();
    if rest.len() >= no_ac.len() && rest[..no_ac.len()].iter().zip(&no_ac).all(|(a, b)| up(*a) == *b as u8) {
        i += no_ac.len();
        // Only a space or the end or a section may follow.
        let tail = &s[i..];
        let mut j = 0;
        while j < tail.len() && is_space(tail[j]) {
            j += 1;
        }
        if j < tail.len() && !(tail.get(j + 1) == Some(&u16::from(b':'))) {
            return Err(SecErr::InvalidParameter);
        }
        return Ok((Section { entries: false, control, acl: None }, i));
    }
    let mut revision = acl::ACL_REVISION;
    let mut body: Vec<u8> = Vec::new();
    let mut count = 0u16;
    let mut slack = 0usize;
    let mut lone = false;
    loop {
        // Between ACEs only spaces; an ACE starts at `(`.
        while i < s.len() && is_space(s[i]) {
            i += 1;
        }
        if i >= s.len() || s[i] != u16::from(b'(') {
            break;
        }
        let start = i + 1;
        if start >= s.len() {
            // A lone `(` at the end is ignored.
            i = s.len();
            lone = true;
            break;
        }
        // The closing `)`: a nested `(` is not a field (but see `ace_end`).
        let Some(end) = ace_end(s, start) else {
            // No closing parenthesis: a bad ACE (too few fields: 87, else bad ACL).
            let f = fields(&s[start..]);
            return Err(if f.len() < 6 { SecErr::InvalidParameter } else { SecErr::InvalidAcl });
        };
        let inner = &s[start..end];
        if inner.is_empty() {
            return Err(SecErr::InvalidParameter);
        }
        let f = fields(inner);
        let (ace, object, slack_here) = parse_ace(&f, sacl)?;
        if object {
            revision = acl::ACL_REVISION_DS;
        }
        slack += slack_here;
        body.extend(ace);
        count += 1;
        i = end + 1;
    }
    let size = acl::ACL_HEADER + body.len() + slack;
    let mut v = Vec::with_capacity(size);
    v.extend_from_slice(&[revision, 0]);
    v.extend_from_slice(&(size as u16).to_le_bytes());
    v.extend_from_slice(&count.to_le_bytes());
    v.extend_from_slice(&[0, 0]);
    v.extend(body);
    v.resize(size, 0);
    Ok((Section { entries: count > 0 || lone, control, acl: Some(v) }, i))
}

/// `ConvertStringSecurityDescriptorToSecurityDescriptorW` (revision 1): the
/// self-relative descriptor.
pub fn parse(s: &[u16]) -> SecResult<Vec<u8>> {
    let mut parts = Parts { revision: sd::SECURITY_DESCRIPTOR_REVISION, ..Parts::default() };
    let (mut have_o, mut have_g, mut have_d, mut have_s) = (false, false, false, false);
    let mut i = 0;
    let mut after_acl = false;
    loop {
        while i < s.len() && is_space(s[i]) {
            i += 1;
        }
        if i >= s.len() {
            break;
        }
        let letter = u8::try_from(s[i]).unwrap_or(0);
        if s.get(i + 1) != Some(&u16::from(b':')) || !matches!(letter, b'O' | b'G' | b'D' | b'S') {
            if after_acl {
                break;
            }
            return Err(SecErr::InvalidParameter);
        }
        i += 2;
        match letter {
            b'O' | b'G' => {
                let seen = if letter == b'O' { &mut have_o } else { &mut have_g };
                if *seen {
                    return Err(SecErr::InvalidParameter);
                }
                *seen = true;
                let (n, bytes) = sid_token(&s[i..])?;
                i += n;
                if letter == b'O' {
                    parts.owner = Some(bytes)
                } else {
                    parts.group = Some(bytes)
                }
                after_acl = false;
            }
            _ => {
                let sacl = letter == b'S';
                let seen = if sacl { &mut have_s } else { &mut have_d };
                if *seen {
                    return Err(SecErr::InvalidParameter);
                }
                *seen = true;
                let (sec, used) = parse_acl_section(&s[i..], sacl)?;
                i += used;
                parts.control |= sec.control;
                if sacl {
                    parts.sacl = sec.acl
                } else {
                    parts.dacl = sec.acl
                }
                after_acl = sec.entries;
            }
        }
    }
    Ok(parts.to_relative())
}

// ----- the other way -----------------------------------------------------------------------

fn sid_text(bytes: &[u8]) -> SecResult<String> {
    let s = Sid::from_bytes(bytes).ok_or(SecErr::InvalidSid)?;
    Ok(sddl::alias_of(&s).map_or_else(|| sddl::to_string(&s), String::from))
}

fn rights_text(mask: u32) -> String {
    use core::fmt::Write;
    if mask == 0 {
        return String::new();
    }
    if let Some((n, _)) = COMPOSITES.iter().find(|(_, v)| *v == mask) {
        return String::from(*n);
    }
    let covered = WRITE_ORDER.iter().fold(0u32, |a, (_, v)| a | v);
    if mask & !covered != 0 {
        let mut s = String::new();
        let _ = write!(s, "0x{mask:x}");
        return s;
    }
    let mut s = String::new();
    for (n, v) in WRITE_ORDER {
        if mask & v != 0 {
            s.push_str(n);
        }
    }
    s
}

fn flags_text(flags: u8, audit: bool) -> String {
    let mut s = String::new();
    for (bit, n) in [(0x01, "OI"), (0x02, "CI"), (0x04, "NP"), (0x08, "IO"), (0x10, "ID"), (0x20, "CR")] {
        if flags & bit != 0 {
            s.push_str(n);
        }
    }
    if audit {
        if flags & acl::SUCCESSFUL_ACCESS_ACE_FLAG != 0 {
            s.push_str("SA");
        }
        if flags & acl::FAILED_ACCESS_ACE_FLAG != 0 {
            s.push_str("FA");
        }
    }
    s
}

fn guid_text(b: &[u8]) -> String {
    use core::fmt::Write;
    let mut s = String::new();
    let d1 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let d2 = u16::from_le_bytes([b[4], b[5]]);
    let d3 = u16::from_le_bytes([b[6], b[7]]);
    let _ = write!(
        s,
        "{d1:08x}-{d2:04x}-{d3:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    );
    s
}

/// The text of one ACE in a DACL (`sacl` false) or SACL; `info` says which
/// SACL ACEs are wanted. `None`: not wanted here.
fn ace_text(a: &acl::Ace, sacl: bool, info: u32) -> SecResult<Option<(String, usize)>> {
    let kind = a.kind;
    let (name, audit, label) = match (kind, sacl) {
        (acl::ACCESS_ALLOWED_ACE_TYPE, false) => ("A", false, false),
        (acl::ACCESS_DENIED_ACE_TYPE, false) => ("D", false, false),
        (acl::ACCESS_ALLOWED_OBJECT_ACE_TYPE, false) => ("OA", false, false),
        (acl::ACCESS_DENIED_OBJECT_ACE_TYPE, false) => ("OD", false, false),
        (acl::SYSTEM_AUDIT_ACE_TYPE, true) => ("AU", true, false),
        (acl::SYSTEM_ALARM_ACE_TYPE, true) => ("AL", true, false),
        (acl::SYSTEM_AUDIT_OBJECT_ACE_TYPE, true) => ("OU", true, false),
        (acl::SYSTEM_ALARM_OBJECT_ACE_TYPE, true) => ("OL", true, false),
        (acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE, true) => ("ML", false, true),
        (acl::ACCESS_ALLOWED_CALLBACK_ACE_TYPE, false) => ("XA", false, false),
        (acl::ACCESS_DENIED_CALLBACK_ACE_TYPE, false) => ("XD", false, false),
        (acl::SYSTEM_AUDIT_CALLBACK_ACE_TYPE, true) => ("XU", true, false),
        (acl::SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE, true) => ("RA", false, false),
        (acl::SYSTEM_SCOPED_POLICY_ID_ACE_TYPE, true) => ("SP", false, false),
        (acl::ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE, false) => return Err(SecErr::CallbackObjectAce),
        _ => return Err(SecErr::InvalidAcl),
    };
    // Which request each kind of SACL entry belongs to.
    let wanted = match kind {
        _ if !sacl => true,
        acl::SYSTEM_MANDATORY_LABEL_ACE_TYPE => info & sd::LABEL_SECURITY_INFORMATION != 0,
        acl::SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE => info & sd::ATTRIBUTE_SECURITY_INFORMATION != 0,
        acl::SYSTEM_SCOPED_POLICY_ID_ACE_TYPE => info & sd::SCOPE_SECURITY_INFORMATION != 0,
        _ => info & sd::SACL_SECURITY_INFORMATION != 0,
    };
    if !wanted {
        return Ok(None);
    }
    let sid = a.sid().ok_or(SecErr::InvalidSid)?;
    let sid_s = sid_text(sid)?;
    let (g1, g2) = if matches!(kind, 5..=8) {
        let of = u32::from_le_bytes([a.bytes[8], a.bytes[9], a.bytes[10], a.bytes[11]]);
        let mut at = 12;
        let mut one = String::new();
        let mut two = String::new();
        if of & 1 != 0 {
            one = guid_text(&a.bytes[at..at + 16]);
            at += 16;
        }
        if of & 2 != 0 {
            two = guid_text(&a.bytes[at..at + 16]);
        }
        (one, two)
    } else {
        (String::new(), String::new())
    };
    let rights = if label {
        let m = a.mask();
        if m & !7 != 0 {
            alloc::format!("0x{m:x}")
        } else {
            let mut s = String::new();
            for (n, v) in LABEL_RIGHTS {
                if m & v != 0 {
                    s.push_str(n);
                }
            }
            s
        }
    } else {
        rights_text(a.mask())
    };
    // What follows the SID: a condition or a resource attribute.
    let after = acl::ACE_FIXED + sid::valid_length(sid).unwrap_or(sid.len());
    let data = a.bytes.get(after..).unwrap_or(&[]);
    let seventh = match kind {
        acl::SYSTEM_RESOURCE_ATTRIBUTE_ACE_TYPE => condace::decompile_attribute(data),
        acl::ACCESS_ALLOWED_CALLBACK_ACE_TYPE
        | acl::ACCESS_DENIED_CALLBACK_ACE_TYPE
        | acl::SYSTEM_AUDIT_CALLBACK_ACE_TYPE => condace::decompile(data),
        _ => None,
    }
    .map(|t| alloc::format!(";({t})"))
    .unwrap_or_default();
    let text = alloc::format!("({name};{};{rights};{g1};{g2};{sid_s}{seventh})", flags_text(a.flags, audit));
    // The size estimate: one more per ACE, two more when the rights are in hexadecimal.
    let extra = 1 + if rights.starts_with("0x") { 10usize.saturating_sub(rights.len()) } else { 0 };
    Ok(Some((text, extra)))
}

/// `ConvertSecurityDescriptorToStringSecurityDescriptorW`: the text and the
/// length the API reports (an upper bound that is larger than the text
/// where it has ACEs).
pub fn format(sd: &Parts, info: u32) -> SecResult<(String, usize)> {
    let mut out = String::new();
    let mut extra = 0usize;
    if info & sd::OWNER_SECURITY_INFORMATION != 0
        && let Some(o) = &sd.owner
    {
        out.push_str("O:");
        out.push_str(&sid_text(o)?);
    }
    if info & sd::GROUP_SECURITY_INFORMATION != 0
        && let Some(g) = &sd.group
    {
        out.push_str("G:");
        out.push_str(&sid_text(g)?);
    }
    let flag_text = |control: u16, p: u16, ar: u16, ai: u16| {
        let mut s = String::new();
        if control & p != 0 {
            s.push('P');
        }
        if control & ar != 0 {
            s.push_str("AR");
        }
        if control & ai != 0 {
            s.push_str("AI");
        }
        s
    };
    let sacl_info = sd::SACL_SECURITY_INFORMATION
        | sd::LABEL_SECURITY_INFORMATION
        | sd::ATTRIBUTE_SECURITY_INFORMATION
        | sd::SCOPE_SECURITY_INFORMATION
        | sd::PROCESS_TRUST_LABEL_SECURITY_INFORMATION;
    for (sacl, wanted) in [(false, info & sd::DACL_SECURITY_INFORMATION != 0), (true, info & sacl_info != 0)] {
        let (present, bytes, p, ar, ai) = if sacl {
            (
                sd::SE_SACL_PRESENT,
                &sd.sacl,
                sd::SE_SACL_PROTECTED,
                sd::SE_SACL_AUTO_INHERIT_REQ,
                sd::SE_SACL_AUTO_INHERITED,
            )
        } else {
            (
                sd::SE_DACL_PRESENT,
                &sd.dacl,
                sd::SE_DACL_PROTECTED,
                sd::SE_DACL_AUTO_INHERIT_REQ,
                sd::SE_DACL_AUTO_INHERITED,
            )
        };
        if !wanted || sd.control & present == 0 {
            continue;
        }
        out.push_str(if sacl { "S:" } else { "D:" });
        out.push_str(&flag_text(sd.control, p, ar, ai));
        match bytes {
            None => {
                out.push_str("NO_ACCESS_CONTROL");
                extra += 1;
            }
            Some(b) => {
                for a in acl::aces_in(b) {
                    if let Some((t, e)) = ace_text(&a, sacl, info)? {
                        out.push_str(&t);
                        extra += e;
                    }
                }
            }
        }
    }
    let len = out.chars().count() + extra;
    Ok((out, len))
}

/// `GENERIC_ALL` is what `GA` stands for; exported for the tests of the
/// crates above.
pub const GA: u32 = GENERIC_ALL;

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn round(s: &str) -> String {
        let rel = parse(&w(s)).unwrap_or_else(|e| panic!("{s}: {e:?}"));
        let p = Parts::from_relative(&rel).unwrap();
        format(&p, 0xFF).unwrap().0
    }

    #[test]
    fn layout_and_sizes() {
        let sizes = [
            ("", 20),
            ("O:BA", 36),
            ("G:SY", 32),
            ("O:BAG:SY", 48),
            ("G:SYO:BA", 48),
            ("D:", 28),
            ("D:NO_ACCESS_CONTROL", 20),
            ("D:(A;;GA;;;WD)", 48),
            ("S:(AU;SAFA;GA;;;WD)", 48),
            ("S:(ML;;NW;;;LW)", 48),
        ];
        for (s, n) in sizes {
            assert_eq!(parse(&w(s)).unwrap().len(), n, "{s}");
        }
        let rel = parse(&w("O:BA")).unwrap();
        assert_eq!(hex(&rel), "010000801400000000000000000000000000000001020000000000052000000020020000");
        let rel = parse(&w("D:(A;;GA;;;WD)")).unwrap();
        assert_eq!(
            &rel[20..],
            [2, 0, 0x1c, 0, 1, 0, 0, 0, 0, 0, 0x14, 0, 0, 0, 0, 0x10, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]
        );
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| alloc::format!("{x:02x}")).collect()
    }

    #[test]
    fn errors() {
        let cases: &[(&str, SecErr)] = &[
            ("O", SecErr::InvalidParameter),
            ("O:", SecErr::NoneMapped),
            ("O:BA;", SecErr::InvalidParameter),
            ("O:BAO:SY", SecErr::InvalidParameter),
            ("D:D:", SecErr::InvalidParameter),
            ("X:BA", SecErr::InvalidParameter),
            ("o:ba", SecErr::InvalidParameter),
            ("O:BAG:", SecErr::NoneMapped),
            ("D:)", SecErr::InvalidParameter),
            ("D:(A", SecErr::InvalidParameter),
            ("D:(A;;GA;;;WD", SecErr::InvalidAcl),
            ("D:((A;;GA;;;WD))", SecErr::InvalidDatatype),
            ("O:BA\n", SecErr::InvalidParameter),
            ("BA", SecErr::InvalidParameter),
            ("O:BAD", SecErr::InvalidParameter),
            ("O:BAD:()", SecErr::InvalidParameter),
            ("O:ZZ", SecErr::InvalidSid),
            ("D:(A;;GA;;;ZZ)", SecErr::InvalidSid),
            ("D:(A;;ZZ;;;WD)", SecErr::InvalidAcl),
            ("D:(A;ZZ;GA;;;WD)", SecErr::InvalidFlags),
            ("D:(ZZ;;GA;;;WD)", SecErr::InvalidDatatype),
            ("O:DA", SecErr::InvalidSid),
            ("D:NULL", SecErr::InvalidParameter),
            ("D:NO_ACCESS_CONTROLP", SecErr::InvalidParameter),
            ("D:Z", SecErr::InvalidParameter),
            ("S:(A;;GA;;;WD)", SecErr::InvalidDatatype),
            ("D:(ML;;NW;;;LW)", SecErr::InvalidDatatype),
            ("D:(OA;;CR;{00299570-246d-11d0-a768-00aa006e0529};;WD)", SecErr::InvalidUuid),
            ("D:(A;;GA;00299570-246d-11d0-a768-00aa006e0529;;WD)", SecErr::InvalidAcl),
            ("D:(A;;0x;;;WD)", SecErr::InvalidAcl),
            ("D:(A;;GA ;;;WD)", SecErr::InvalidAcl),
            ("D:(A;SAFA;GA;;;WD)", SecErr::InvalidFlags),
        ];
        for (s, e) in cases {
            assert_eq!(parse(&w(s)), Err(*e), "{s}");
        }
        for ok in [
            "O:BA ",
            " O:BA",
            "O:BA G:SY",
            "D:(",
            "D:(A;;GA;;;WD))",
            "D:(A;;GA;;;WD)(",
            "D:(A;;GA;;;WD)P",
            "D:(A;;GA;;;WD)extra",
            " ",
            "D: (A;;GA;;;WD)",
            "D:(A;; GA;;; WD )",
            "D:(a;;ga;;;wd)",
        ] {
            parse(&w(ok)).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
        }
    }

    #[test]
    fn rights_and_text() {
        let m = |s: &str| {
            let rel = parse(&w(&alloc::format!("D:(A;;{s};;;WD)"))).unwrap();
            let p = Parts::from_relative(&rel).unwrap();
            // SAFETY: the bytes are a whole ACL the parser built.
            let a = unsafe { acl::aces(acl::AclPtr(p.dacl.unwrap().as_ptr() as *mut u8)) };
            a[0].mask()
        };
        assert_eq!(m("GAGR"), 0x9000_0000);
        assert_eq!(m("FRFW"), 0x12019f);
        assert_eq!(m("0x1GA"), 0x1000_0001);
        assert_eq!(m("GA0x1"), 0x1000_0001);
        assert_eq!(m("-1"), 0xffff_ffff);
        assert_eq!(m("0x1FFFFFFFFF"), 0xffff_ffff);
        assert_eq!(m("123"), 0x7b);
        assert_eq!(m("ga"), 0x1000_0000);
        for (a, b) in [
            ("D:(A;;GA;;;WD)", "D:(A;;GA;;;WD)"),
            ("D:(A;;0x1f01ff;;;S-1-1-0)", "D:(A;;FA;;;WD)"),
            ("D:(A;;GRGWGX;;;WD)", "D:(A;;GXGWGR;;;WD)"),
            ("D:(A;;RCSDWDWO;;;WD)", "D:(A;;SDRCWDWO;;;WD)"),
            ("D:(A;;CRLO;;;WD)", "D:(A;;LOCR;;;WD)"),
            ("D:(A;;FRFW;;;WD)", "D:(A;;0x12019f;;;WD)"),
            ("D:(A;;KRKW;;;WD)", "D:(A;;CCDCLCSWRPRC;;;WD)"),
            ("D:(A;CIOI;GA;;;WD)", "D:(A;OICI;GA;;;WD)"),
            ("D:AIP(A;;GA;;;WD)", "D:PAI(A;;GA;;;WD)"),
            ("S:(AU;FASA;GA;;;WD)", "S:(AU;SAFA;GA;;;WD)"),
            ("S:(ML;;NXNRNW;;;S-1-16-4096)", "S:(ML;;NWNRNX;;;LW)"),
            ("O:BAG:SYD:NO_ACCESS_CONTROL", "O:BAG:SYD:NO_ACCESS_CONTROL"),
            (
                "D:(OA;;CR;00299570-246d-11d0-a768-00aa006e0529;;WD)",
                "D:(OA;;CR;00299570-246d-11d0-a768-00aa006e0529;;WD)",
            ),
            ("D:(OA;;CR;;;WD)", "D:(A;;CR;;;WD)"),
        ] {
            assert_eq!(round(a), b, "{a}");
        }
        let rel = parse(&w("D:(OA;;CR;00299570-246d-11d0-a768-00aa006e0529;;WD)")).unwrap();
        assert_eq!(rel[20], 4);
        assert_eq!(hex(&rel[28..32]), "05002800");
    }

    #[test]
    fn lengths() {
        for (s, n) in [
            ("D:(A;;GA;;;WD)", 15),
            ("D:NO_ACCESS_CONTROL", 20),
            ("D:(A;;0x12019f;;;WD)", 23),
            ("D:(A;;CCDCLCSWRPRC;;;WD)", 25),
            ("O:BA", 4),
        ] {
            let rel = parse(&w(s)).unwrap();
            let p = Parts::from_relative(&rel).unwrap();
            let (t, len) = format(&p, 0xFF).unwrap();
            assert_eq!(len, n, "{s} -> {t}");
        }
    }
}
