//! The one piece of AML Poly reads without an interpreter: the sleep type
//! values of a sleeping state (`\_S5` for soft-off), which the firmware
//! declares in the DSDT as a named package (ACPI 6.5 §7.4.2, "\_Sx (System
//! States)"), and the PM1 control value that enters it (§4.8.3.2.1, §16.1).
//!
//! The encoding is the AML grammar of §20.2: `DefName := NameOp NameString
//! DataRefObject`, `DefPackage := PackageOp PkgLength NumElements
//! PackageElementList`, with integers as `ZeroOp`, `OneOp`, `OnesOp` or a
//! `BytePrefix` / `WordPrefix` / `DWordPrefix` / `QWordPrefix` constant.
//! The search looks for the name's four bytes preceded by `NameOp` (with
//! an optional root or parent prefix) and followed by a package; that is
//! how every firmware the kernel boots on (QEMU's included) declares it.
//! A declaration computed by a method or hidden in a conditional is not
//! found, and the caller then cannot enter the state.

const NAME_OP: u8 = 0x08;
const PACKAGE_OP: u8 = 0x12;
const ZERO_OP: u8 = 0x00;
const ONE_OP: u8 = 0x01;
const ONES_OP: u8 = 0xff;
const BYTE_PREFIX: u8 = 0x0a;
const WORD_PREFIX: u8 = 0x0b;
const DWORD_PREFIX: u8 = 0x0c;
const QWORD_PREFIX: u8 = 0x0e;
const ROOT_CHAR: u8 = b'\\';
const PARENT_PREFIX: u8 = b'^';

/// `SLP_TYP` (bits 10-12) and `SLP_EN` (bit 13) of the PM1 control
/// registers (§4.8.3.2.1, Table 4.13).
pub const SLP_TYP_SHIFT: u32 = 10;
pub const SLP_TYP_MASK: u16 = 0b111 << SLP_TYP_SHIFT;
pub const SLP_EN: u16 = 1 << 13;
/// `SCI_EN` (bit 0): the machine is in ACPI mode.
pub const SCI_EN: u16 = 1;

/// The `SLP_TYPa` and `SLP_TYPb` values of sleeping state `state` (0 to 5),
/// read from the AML table body `aml` (the DSDT or an SSDT, without its
/// header). `None` if the table does not declare `\_Sx` as a package of at
/// least two integers.
pub fn sleep_types(aml: &[u8], state: u8) -> Option<(u8, u8)> {
    if state > 5 {
        return None;
    }
    let name = [b'_', b'S', b'0' + state, b'_'];
    let mut from = 0;
    while let Some(at) = find(&aml[from..], &name).map(|i| i + from) {
        from = at + 1;
        if !preceded_by_name_op(aml, at) {
            continue;
        }
        if let Some(types) = package_types(aml.get(at + 4..)?) {
            return Some(types);
        }
    }
    None
}

fn find(hay: &[u8], needle: &[u8; 4]) -> Option<usize> {
    hay.windows(4).position(|w| w == needle)
}

/// `NameOp`, then perhaps a root character or parent prefixes, then the
/// name at `at`.
fn preceded_by_name_op(aml: &[u8], at: usize) -> bool {
    let mut i = at;
    while i > 0 && matches!(aml[i - 1], ROOT_CHAR | PARENT_PREFIX) {
        i -= 1;
    }
    i > 0 && aml[i - 1] == NAME_OP
}

/// The first two integers of the package `b` starts with.
fn package_types(b: &[u8]) -> Option<(u8, u8)> {
    if *b.first()? != PACKAGE_OP {
        return None;
    }
    let lead = *b.get(1)?;
    let follow = usize::from(lead >> 6);
    // PkgLength (§20.2.4) counts itself; only its size matters here.
    let mut i = 2 + follow;
    let count = *b.get(i)?;
    i += 1;
    if count < 2 {
        return None;
    }
    let (a, n) = integer(b.get(i..)?)?;
    i += n;
    let (bb, _) = integer(b.get(i..)?)?;
    Some((a as u8, bb as u8))
}

/// An integer data object and the bytes it takes.
fn integer(b: &[u8]) -> Option<(u64, usize)> {
    let le = |n: usize| -> Option<(u64, usize)> {
        let bytes = b.get(1..1 + n)?;
        Some((bytes.iter().rev().fold(0u64, |acc, &x| acc << 8 | u64::from(x)), 1 + n))
    };
    match *b.first()? {
        ZERO_OP => Some((0, 1)),
        ONE_OP => Some((1, 1)),
        ONES_OP => Some((u64::MAX, 1)),
        BYTE_PREFIX => le(1),
        WORD_PREFIX => le(2),
        DWORD_PREFIX => le(4),
        QWORD_PREFIX => le(8),
        _ => None,
    }
}

/// The value to write to a PM1 control register that reads `current` to
/// enter the sleeping state with sleep type `slp_typ`: its other bits
/// kept, `SLP_TYP` set and `SLP_EN` (§4.8.3.2.1: both in one write).
pub fn sleep_control(current: u16, slp_typ: u8) -> u16 {
    (current & !(SLP_TYP_MASK | SLP_EN)) | ((u16::from(slp_typ) & 0b111) << SLP_TYP_SHIFT) | SLP_EN
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Name (_S5, Package (0x04) { 0x05, 0x05, Zero, Zero })` and a
    /// distraction before it.
    const DSDT: &[u8] = &[
        0x10,
        0x05,
        b'_',
        b'S',
        b'B',
        b'_', // Scope (_SB_) header bytes
        b'_',
        b'S',
        b'5',
        b'_',
        0x00, // the name without NameOp: not a declaration
        NAME_OP,
        b'_',
        b'S',
        b'3',
        b'_',
        PACKAGE_OP,
        0x06,
        0x04,
        BYTE_PREFIX,
        0x01,
        ONE_OP,
        ZERO_OP,
        ZERO_OP,
        NAME_OP,
        ROOT_CHAR,
        b'_',
        b'S',
        b'5',
        b'_',
        PACKAGE_OP,
        0x08,
        0x04,
        BYTE_PREFIX,
        0x05,
        BYTE_PREFIX,
        0x05,
        ZERO_OP,
        ZERO_OP,
    ];

    #[test]
    fn finds_the_sleep_types() {
        assert_eq!(sleep_types(DSDT, 5), Some((5, 5)));
        assert_eq!(sleep_types(DSDT, 3), Some((1, 1)));
        assert_eq!(sleep_types(DSDT, 4), None);
        assert_eq!(sleep_types(DSDT, 9), None);
    }

    #[test]
    fn packages_with_zero_one_and_long_lengths() {
        // A two-byte PkgLength and ZeroOp / OneOp elements.
        let aml = [NAME_OP, b'_', b'S', b'5', b'_', PACKAGE_OP, 0x40, 0x00, 0x02, ZERO_OP, ONE_OP];
        assert_eq!(sleep_types(&aml, 5), Some((0, 1)));
        // A package of one element, or something other than a package.
        let short = [NAME_OP, b'_', b'S', b'5', b'_', PACKAGE_OP, 0x03, 0x01, ZERO_OP];
        assert_eq!(sleep_types(&short, 5), None);
        let not_package = [NAME_OP, b'_', b'S', b'5', b'_', BYTE_PREFIX, 0x05];
        assert_eq!(sleep_types(&not_package, 5), None);
        // Cut off anywhere: never a panic.
        for n in 0..DSDT.len() {
            let _ = sleep_types(&DSDT[..n], 5);
        }
    }

    #[test]
    fn the_control_value_keeps_other_bits() {
        assert_eq!(sleep_control(SCI_EN, 5), SCI_EN | (5 << 10) | SLP_EN);
        assert_eq!(sleep_control(0xffff, 0), !SLP_TYP_MASK);
        assert_eq!(sleep_control(0, 7), (7 << 10) | SLP_EN);
    }
}
