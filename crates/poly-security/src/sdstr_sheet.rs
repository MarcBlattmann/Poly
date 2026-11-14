//! The statements of the two SDDL spec sheets
//! (`specs/windows/advapi32.dll/Convert*SecurityDescriptor*W.yaml`), one
//! assertion per statement, so a gap between the sheet and the code shows
//! up on the host.

use crate::acl;
use crate::err::SecErr;
use crate::sd::Parts;
use crate::sddl;
use crate::sdstr::{format, parse};
use crate::sid::Sid;
use alloc::string::String;
use alloc::vec::Vec;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// The descriptor, or the Win32 error.
fn p(s: &str) -> Result<Vec<u8>, u32> {
    parse(&w(s)).map_err(SecErr::win32)
}

fn ok(s: &str) -> Vec<u8> {
    p(s).unwrap_or_else(|e| panic!("{s:?}: error {e}"))
}

fn dword(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

fn control(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[2], b[3]])
}

/// (owner, group, sacl, dacl) offsets.
fn offsets(b: &[u8]) -> (usize, usize, usize, usize) {
    (dword(b, 4), dword(b, 8), dword(b, 12), dword(b, 16))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| alloc::format!("{x:02x}")).collect()
}

/// The owner of `O:<sid>` as a SID string.
fn owner(s: &str) -> String {
    let b = ok(&alloc::format!("O:{s}"));
    sddl::to_string(&Sid::from_bytes(&b[20..]).unwrap_or_else(|| panic!("{s}: not a valid SID")))
}

/// The ACEs of the DACL (`sacl` false) or SACL of a descriptor.
fn aces(b: &[u8], sacl: bool) -> Vec<acl::Ace> {
    let (_, _, s, d) = offsets(b);
    let at = if sacl { s } else { d };
    assert_ne!(at, 0, "no ACL");
    acl::aces_in(&b[at..])
}

/// The first DACL ACE of `D:(<ace>)`.
fn dace(ace: &str) -> acl::Ace {
    aces(&ok(&alloc::format!("D:({ace})")), false).remove(0)
}

fn sace(ace: &str) -> acl::Ace {
    aces(&ok(&alloc::format!("S:({ace})")), true).remove(0)
}

fn mask(rights: &str) -> u32 {
    dace(&alloc::format!("A;;{rights};;;WD")).mask()
}

fn text(sd: &[u8], info: u32) -> Result<String, u32> {
    format(&Parts::from_relative(sd).expect("a descriptor"), info).map(|t| t.0).map_err(SecErr::win32)
}

fn round(s: &str) -> String {
    text(&ok(s), 0xff).unwrap_or_else(|e| panic!("{s}: error {e}"))
}

fn length(s: &str) -> usize {
    format(&Parts::from_relative(&ok(s)).unwrap(), 0xff).unwrap().1
}

// ----- ConvertStringSecurityDescriptorToSecurityDescriptorW -------------------------------

#[test]
fn parse_layout() {
    assert_eq!(ok("G:SYO:BA"), ok("O:BAG:SY"));
    let b = ok("O:BAG:SY");
    assert_eq!((b.len(), offsets(&b)), (48, (20, 36, 0, 0)));
    let b = ok("O:BAG:SYD:(A;;GA;;;WD)");
    assert_eq!((b.len(), offsets(&b)), (76, (48, 64, 0, 20)));
    assert!(hex(&b).starts_with("010004803000000040000000000000001400000002001c0001000000"));
    let b = ok("O:BAG:SYD:(A;;GA;;;WD)S:(AU;SAFA;GA;;;WD)");
    assert_eq!((b.len(), offsets(&b)), (104, (76, 92, 20, 48)));
    assert_eq!(hex(&ok("")), "0100008000000000000000000000000000000000");
    let b = ok("D:");
    assert_eq!((b.len(), offsets(&b).3, hex(&b[20..])), (28, 20, String::from("0200080000000000")));
    let b = ok("D:NO_ACCESS_CONTROL");
    assert_eq!((b.len(), control(&b), offsets(&b).3), (20, 0x8004, 0));
    let b = ok("S:");
    assert_eq!((b.len(), control(&b)), (28, 0x8010));
}

#[test]
fn parse_sid_aliases() {
    let aliases = [
        ("AA", "S-1-5-32-579"),
        ("AC", "S-1-15-2-1"),
        ("AN", "S-1-5-7"),
        ("AO", "S-1-5-32-548"),
        ("AS", "S-1-18-1"),
        ("AU", "S-1-5-11"),
        ("BA", "S-1-5-32-544"),
        ("BG", "S-1-5-32-546"),
        ("BO", "S-1-5-32-551"),
        ("BU", "S-1-5-32-545"),
        ("CD", "S-1-5-32-574"),
        ("CG", "S-1-3-1"),
        ("CO", "S-1-3-0"),
        ("CY", "S-1-5-32-569"),
        ("ED", "S-1-5-9"),
        ("ER", "S-1-5-32-573"),
        ("ES", "S-1-5-32-576"),
        ("HA", "S-1-5-32-578"),
        ("HI", "S-1-16-12288"),
        ("IS", "S-1-5-32-568"),
        ("IU", "S-1-5-4"),
        ("LS", "S-1-5-19"),
        ("LU", "S-1-5-32-559"),
        ("LW", "S-1-16-4096"),
        ("ME", "S-1-16-8192"),
        ("MP", "S-1-16-8448"),
        ("MS", "S-1-5-32-577"),
        ("MU", "S-1-5-32-558"),
        ("NO", "S-1-5-32-556"),
        ("NS", "S-1-5-20"),
        ("NU", "S-1-5-2"),
        ("OW", "S-1-3-4"),
        ("PO", "S-1-5-32-550"),
        ("PS", "S-1-5-10"),
        ("PU", "S-1-5-32-547"),
        ("RA", "S-1-5-32-575"),
        ("RC", "S-1-5-12"),
        ("RD", "S-1-5-32-555"),
        ("RE", "S-1-5-32-552"),
        ("RM", "S-1-5-32-580"),
        ("RU", "S-1-5-32-554"),
        ("SI", "S-1-16-16384"),
        ("SO", "S-1-5-32-549"),
        ("SS", "S-1-18-2"),
        ("SU", "S-1-5-6"),
        ("SY", "S-1-5-18"),
        ("UD", "S-1-5-84-0-0-0-0-0"),
        ("WD", "S-1-1-0"),
        ("WR", "S-1-5-33"),
    ];
    for (a, s) in aliases {
        assert_eq!(owner(a), s, "{a}");
        assert_eq!(owner(&a.to_ascii_lowercase()), s, "{a} lower");
    }
    let machine = sddl::to_string(&crate::sid::domain_relative(500));
    assert_eq!(owner("LA"), machine);
    assert!(owner("LG").ends_with("-501"));
    assert_eq!(ok("O:LA").len(), 48);
    assert_eq!(dace("A;;GA;;;LA").bytes.len(), 36);
    assert_eq!(dace("A;;GA;;;LG").bytes.len(), 36);
    assert_eq!(ok("G:LG").len(), 48);
}

#[test]
fn parse_domain_aliases() {
    for a in [
        "AP", "CA", "CN", "DA", "DC", "DD", "DG", "DU", "EA", "EK", "KA", "PA", "RO", "RS", "SA", "AD", "BT", "CL",
        "DI", "IL", "LN", "NA", "OL", "SV", "TS", "WP", "XX", "ZZ",
    ] {
        assert_eq!(p(&alloc::format!("O:{a}")), Err(1337), "O:{a}");
        assert_eq!(p(&alloc::format!("G:{a}")), Err(1337), "G:{a}");
    }
    assert_eq!(p("D:(A;;GA;;;DA)"), Err(1337));
}

#[test]
fn parse_numeric_sids() {
    for s in ["S-1-5-18", "S-1-5-32-544", "S-1-1-0", "S-1-0-0", "S-1-3-4", "S-1-16-4096"] {
        assert_eq!(owner(s), s);
    }
    assert_eq!(ok("O:S-1-5-21-1-2-3").len(), 44);
    assert_eq!(ok("O:S-1-5-21-1-2-3-500").len(), 48);
    assert_eq!(owner("S-1-0x10-1"), "S-1-16-1");
    assert_eq!(owner("S-1-5-0x12"), "S-1-5-18");
    assert_eq!(owner("S-1-0x5-18"), "S-1-5-18");
    assert_eq!(owner("S-1-5-4294967295"), "S-1-5-4294967295");
    assert_eq!(owner("S-1-5-4294967296"), "S-1-5-4294967295");
    assert_eq!(owner("s-1-5-18"), "S-1-5-18");
    for s in ["S-2-5-18", "S-0-5-18"] {
        assert_eq!(ok(&alloc::format!("O:{s}")).len(), 32, "{s}");
    }
    assert_eq!(ok(&alloc::format!("O:S-1-5{}", "-1".repeat(16))).len(), 92);
    assert_eq!(ok(&alloc::format!("O:S-1-5{}", "-1".repeat(17))).len(), 96);
    for s in ["S-1-5", "S-1", "S-1-", "S-1-5-", "S-1-5-18-", "S-1-5-18x", "B A", "(BA)"] {
        assert_eq!(p(&alloc::format!("O:{s}")), Err(1337), "{s}");
    }
    assert_eq!(p("O:BAA"), Err(87));
    // A numeric SID ends where the next part starts, whatever its letter.
    for (s, n) in [
        ("O:S-1-5-18G:S-1-5-32-544", 20 + 12 + 16),
        ("O:S-1-5-18D:(A;;GA;;;WD)", 20 + 12 + 28),
        ("O:S-1-5-32-544G:S-1-5-18S:(ML;;NW;;;LW)", 20 + 16 + 12 + 28),
        ("G:S-1-5-18D:", 20 + 12 + 8),
        ("O:S-1-5-0x12D:", 20 + 12 + 8),
    ] {
        assert_eq!(p(s).map(|b| b.len()), Ok(n), "{s}");
    }
    assert_eq!(dace("A;;GA;;;S-1-5-32-544").bytes.len(), 24);
}

#[test]
fn parse_rights() {
    let rights = [
        ("GA", 0x1000_0000u32),
        ("GR", 0x8000_0000),
        ("GW", 0x4000_0000),
        ("GX", 0x2000_0000),
        ("FA", 0x1f01ff),
        ("FR", 0x120089),
        ("FW", 0x120116),
        ("FX", 0x1200a0),
        ("KA", 0xf003f),
        ("KR", 0x20019),
        ("KW", 0x20006),
        ("KX", 0x20019),
        ("RC", 0x20000),
        ("SD", 0x10000),
        ("WD", 0x40000),
        ("WO", 0x80000),
        ("CC", 1),
        ("DC", 2),
        ("LC", 4),
        ("SW", 8),
        ("RP", 0x10),
        ("WP", 0x20),
        ("DT", 0x40),
        ("LO", 0x80),
        ("CR", 0x100),
        ("GAGR", 0x9000_0000),
        ("RCSDWDWO", 0xf0000),
        ("CCDCLCSWRPWPDTLOCR", 0x1ff),
        ("FRFW", 0x12019f),
        ("SDSD", 0x10000),
        ("0x1", 1),
        ("0X10", 0x10),
        ("0xFFFFFFFF", 0xffff_ffff),
        ("0x1F01FF", 0x1f01ff),
        ("0xabc", 0xabc),
        ("123", 0x7b),
        ("0", 0),
        ("01", 1),
        ("-1", 0xffff_ffff),
        ("0x1FFFFFFFF", 0xffff_ffff),
        ("0x100000000", 0xffff_ffff),
        ("0x1GA", 0x1000_0001),
        ("GA0x1", 0x1000_0001),
        ("ga", 0x1000_0000),
        (" GA", 0x1000_0000),
        ("", 0),
    ];
    for (s, m) in rights {
        assert_eq!(mask(s), m, "{s:?}");
    }
    for s in ["0x", "0xZ", "0x1G", "GAZZ", "ZZ", "G", "GAG", "R", "RW", "AX", "0x-1", "0x 1", "GA "] {
        assert_eq!(p(&alloc::format!("D:(A;;{s};;;WD)")), Err(1336), "{s:?}");
    }
}

#[test]
fn parse_ace_types() {
    assert_eq!(dace("A;;GA;;;WD").kind, 0);
    assert_eq!(dace("D;;GA;;;WD").kind, 1);
    assert_eq!(sace("AU;SA;GA;;;WD").kind, 2);
    assert_eq!(p("S:(AL;SA;GA;;;WD)"), Err(87));
    assert_eq!(p("D:(AL;;GA;;;WD)"), Err(1336));
    for (r, m) in [("NW", 1), ("NR", 2), ("NX", 4), ("NWNRNX", 7), ("NWNW", 1), ("", 0), ("0x1", 1)] {
        let a = sace(&alloc::format!("ML;;{r};;;LW"));
        assert_eq!((a.kind, a.mask()), (0x11, m), "{r}");
    }
    assert_eq!(p("S:(ML;;GA;;;LW)"), Err(1336));
    assert_eq!(p("S:(ML;;NW;;;WD)"), Err(87));
    assert_eq!(sace("ML;CI;NW;;;LW").flags, 2);
    assert_eq!(sace("ML;OICI;NW;;;LW").flags, 3);
    for f in ["CI", "OICI", "IO", "ID"] {
        assert_eq!(p(&alloc::format!("S:(ML;{f};GA;;;LW)")), Err(1336), "{f}");
    }
    // Conditional ACEs.
    let a = dace("XA;;GA;;;WD;(Member_of {SID(BA)})");
    assert_eq!((a.kind, a.bytes.len()), (9, 52));
    assert_eq!(hex(&a.bytes[20..]), "6172747850150000005110000000010200000000000520000000200200008900");
    let a = dace("XD;;GA;;;WD;(Member_of {SID(BA)})");
    assert_eq!((a.kind, a.bytes.len()), (0xa, 52));
    assert_eq!(round("D:(XA;;GA;;;WD;(Member_of {SID(BA)}))"), "D:(XA;;GA;;;WD;(Member_of {SID(BA)}))");
    assert_eq!(round("D:(XD;;GA;;;WD;(Member_of {SID(BA)}))"), "D:(XD;;GA;;;WD;(Member_of {SID(BA)}))");
    let a = sace("XU;SA;GA;;;WD;(Member_of {SID(BA)})");
    assert_eq!((a.kind, a.bytes.len(), a.flags), (0xd, 52, 0x40));
    assert_eq!(sace("RA;;;;;WD;(\"Project\",TS,0,\"Poly\")").kind, 0x12);
    let a = sace("SP;;;;;S-1-17-1");
    assert_eq!((a.kind, a.bytes.len()), (0x13, 20));
    assert_eq!(p("D:(ZA;;GA;;;WD)"), Err(1336));
    assert_eq!(p("D:(AA;;GA;;;WD)"), Err(1336));
    assert_eq!(p("D:(TL;;GA;;;WD)"), Err(87));
    for t in ["X", "ZZ"] {
        assert_eq!(p(&alloc::format!("D:({t};;GA;;;WD)")), Err(1804), "{t}");
    }
    assert_eq!(p("S:(A;;GA;;;WD)"), Err(1804));
    assert_eq!(p("S:(D;;GA;;;WD)"), Err(1804));
    assert_eq!(p("D:(ML;;NW;;;LW)"), Err(1804));
    assert_eq!(p("D:(AU;SA;GA;;;WD)"), Err(1336));
    assert_eq!(dace("a;;ga;;;wd").mask(), 0x1000_0000);
}

#[test]
fn parse_ace_bytes() {
    let b = ok("D:(A;;GA;;;WD)");
    assert_eq!(hex(&b[20..]), "02001c00010000000000140000000010010100000000000100000000");
    let b = ok("S:(ML;;NW;;;LW)");
    assert_eq!(hex(&b[28..]), "1100140001000000010100000000001000100000");
    let b = ok("D:(A;;GA;;;WD)(D;;GW;;;BA)(A;;GR;;;SY)");
    assert_eq!(
        (b[20], dword(&b, 20) >> 16, aces(&b, false).iter().map(|a| a.bytes.len()).collect::<Vec<_>>()),
        (2, 72, alloc::vec![20, 24, 20])
    );
}

#[test]
fn parse_object_aces() {
    let g = "00299570-246d-11d0-a768-00aa006e0529";
    let b = ok(&alloc::format!("D:(OA;;CR;{g};;WD)"));
    let a = &aces(&b, false)[0];
    assert_eq!((b[20], dword(&b, 20) >> 16, a.kind, a.bytes.len(), a.mask()), (4, 48, 5, 40, 0x100));
    assert_eq!(hex(&a.bytes[8..28]), "01000000709529006d24d011a76800aa006e0529");
    let b = ok(&alloc::format!("D:(OA;;CR;{g};{g};WD)"));
    let a = &aces(&b, false)[0];
    assert_eq!((dword(&b, 20) >> 16, a.bytes.len(), dword(&a.bytes, 8)), (64, 56, 3));
    let a = dace(&alloc::format!("OA;;CR;;{g};WD"));
    assert_eq!((a.bytes.len(), dword(&a.bytes, 8)), (40, 2));
    assert_eq!(dace(&alloc::format!("OD;;CR;{g};;WD")).kind, 6);
    let b = ok("D:(OA;;CR;;;WD)");
    assert_eq!((b[20], aces(&b, false)[0].kind), (4, 0));
    for bad in [alloc::format!("{{{g}}}"), String::from("00299570-246d-11d0-a768-00aa006e052"), String::from("xyz")] {
        assert_eq!(p(&alloc::format!("D:(OA;;CR;{bad};;WD)")), Err(1705), "{bad}");
        assert_eq!(p(&alloc::format!("S:(OU;SA;CR;{bad};;WD)")), Err(1705), "OU {bad}");
        assert_eq!(p(&alloc::format!("S:(OL;SA;CR;{bad};;WD)")), Err(1705), "OL {bad}");
    }
    assert_eq!(p(&alloc::format!("D:(A;;CR;{g};;WD)")), Err(1336));
}

#[test]
fn parse_ace_flags() {
    let f = |s: &str| dace(&alloc::format!("A;{s};GA;;;WD")).flags;
    for (s, v) in [
        ("CI", 2),
        ("OI", 1),
        ("NP", 4),
        ("IO", 8),
        ("ID", 0x10),
        ("OIOI", 1),
        ("CIOI", 3),
        ("IOCIOI", 0xb),
        ("CIOINPIOID", 0x1f),
        ("cioi", 3),
        ("CI OI", 3),
    ] {
        assert_eq!(f(s), v, "{s}");
    }
    let f = |s: &str| sace(&alloc::format!("AU;{s};GA;;;WD")).flags;
    for (s, v) in [("SA", 0x40), ("FA", 0x80), ("SAFA", 0xc0), ("FASA", 0xc0), ("SAFAOICI", 0xc3), ("IOSA", 0x48)] {
        assert_eq!(f(s), v, "{s}");
    }
    for s in ["XX", "CIXX", "0x1", "SA", "FA", "SAFA"] {
        assert_eq!(p(&alloc::format!("D:(A;{s};GA;;;WD)")), Err(1004), "{s}");
    }
}

#[test]
fn parse_acl_flags() {
    for (s, c) in [
        ("D:P", 0x9004u16),
        ("D:AR", 0x8104),
        ("D:AI", 0x8404),
        ("D:AIP", 0x9404),
        ("D:PARAI", 0x9504),
        ("D:PP", 0x9004),
        ("D:AIAI", 0x8404),
        ("D:p", 0x9004),
        ("D:aip", 0x9404),
        ("S:P", 0xa010),
        ("S:AR", 0x8210),
        ("S:AI", 0x8810),
        ("S:PAI", 0xa810),
        ("D:(A;;GA;;;WD)P", 0x8004),
        ("D:PNO_ACCESS_CONTROL", 0x9004),
        ("D:AINO_ACCESS_CONTROL", 0x8404),
    ] {
        assert_eq!(control(&ok(s)), c, "{s}");
    }
    let b = ok("D:P");
    assert_eq!((b.len(), offsets(&b).3), (28, 20));
    let b = ok("D:PNO_ACCESS_CONTROL");
    assert_eq!((b.len(), offsets(&b).3), (20, 0));
    let b = ok("S:NO_ACCESS_CONTROL");
    assert_eq!((b.len(), control(&b), offsets(&b).2), (20, 0x8010, 0));
    for s in ["D:NULL", "D:NO_ACCESS_CONTROLP", "D:NO_ACCESS_CONTROL(A;;GA;;;WD)", "D:Z", "D:PZ"] {
        assert_eq!(p(s), Err(87), "{s}");
    }
}

#[test]
fn parse_string_syntax() {
    assert_eq!(ok("O:BA ").len(), 36);
    assert_eq!(ok(" O:BA").len(), 36);
    assert_eq!(ok("O:BA G:SY").len(), 48);
    for s in [" ", "  ", ""] {
        assert_eq!(ok(s).len(), 20);
    }
    for s in ["O:BA\n", "O:BA\t", "O:BAG:SY\n", "\n", "\t"] {
        assert_eq!(p(s), Err(87), "{s:?}");
    }
    for s in ["D:(A;;GA;;;WD)extra", "D:(A;;GA;;;WD))", "D:(A;;GA;;;WD)("] {
        let b = ok(s);
        assert_eq!((b.len(), aces(&b, false).len()), (48, 1), "{s}");
    }
    for s in ["D:(", "O:BAD:("] {
        let b = ok(s);
        assert_eq!(dword(&b, offsets(&b).3), 0x0008_0002, "{s}");
    }
    for s in ["D: (A;;GA;;;WD)", "D:( A;;GA;;;WD)", "D:(A;; GA;;;WD)", "D:(A;;GA;;; WD)", "D:(A;;GA;;;WD )"] {
        assert_eq!(ok(s), ok("D:(A;;GA;;;WD)"), "{s}");
    }
}

#[test]
fn parse_errors() {
    for (s, e) in [
        ("O", 87),
        ("O:BA;", 87),
        ("O:BAO:SY", 87),
        ("G:SYG:BA", 87),
        ("D:D:", 87),
        ("S:S:", 87),
        ("D:(A;;GA;;;WD)D:(A;;GA;;;WD)", 87),
        ("X:BA", 87),
        ("T:BA", 87),
        ("L:BA", 87),
        ("o:ba", 87),
        ("BA", 87),
        ("O:BAD", 87),
        ("O:BA:", 87),
        ("D:)", 87),
        ("D:(A", 87),
        ("D:()", 87),
        ("D:(A;;GA;;WD)", 87),
        ("D:(A;;GA;;;;WD)", 87),
        ("D:(A;GA;WD)", 87),
        ("O:", 1332),
        ("G:", 1332),
        ("O:BAG:", 1332),
        ("O:ZZ", 1337),
        ("G:S-1", 1337),
        ("D:(A;;GA;;;ZZ)", 1337),
        ("D:(A;;GA;;;)", 1337),
        ("D:(A;;ZZ;;;WD)", 1336),
        ("D:(A;;GA;;;WD", 1336),
        ("D:(AA;;GA;;;WD)", 1336),
        ("D:((A;;GA;;;WD))", 1804),
        ("S:(A;;GA;;;WD)", 1804),
        ("D:(A;ZZ;GA;;;WD)", 1004),
    ] {
        assert_eq!(p(s), Err(e), "{s}");
    }
}

/// The size of the descriptor or the error, for a table of cases.
fn outcome(s: &str) -> Result<usize, u32> {
    p(s).map(|b| b.len())
}

fn check(cases: &[(&str, Result<usize, u32>)]) {
    for (s, want) in cases {
        assert_eq!(outcome(s), *want, "{s:?}");
    }
}

const G1: &str = "6da8a4ff-0e52-11d0-a286-00aa003049e2";
const G2: &str = "e45795b2-9455-11d1-aebd-0000f80367c1";

#[test]
fn parse_alarm_types() {
    for (ty, plain, one, both, dacl) in [
        ("AU", Ok(52), Err(1336), Err(1336), 1336),
        ("AL", Err(87), Err(1336), Err(1336), 1336),
        ("OU", Ok(56), Ok(68), Ok(84), 1804),
        ("OL", Err(87), Err(87), Err(87), 1804),
        ("al", Err(87), Err(1336), Err(1336), 1336),
        ("Ol", Err(87), Err(87), Err(87), 1804),
    ] {
        assert_eq!(outcome(&alloc::format!("S:({ty};FA;GR;;;BA)")), plain, "{ty}");
        assert_eq!(outcome(&alloc::format!("S:({ty};;GR;;;BA)")), plain, "{ty}");
        assert_eq!(outcome(&alloc::format!("S:({ty};SAFA;CR;{G1};;SY)")), one, "{ty}");
        assert_eq!(outcome(&alloc::format!("S:({ty};FA;CR;;{G2};SY)")), one, "{ty}");
        assert_eq!(outcome(&alloc::format!("S:({ty};SA;CR;{G1};{G2};SY)")), both, "{ty}");
        assert_eq!(outcome(&alloc::format!("D:({ty};;GR;;;BA)")), Err(dacl), "{ty}");
        assert_eq!(outcome(&alloc::format!("D:({ty};SA;CR;{G1};;BA)")), Err(dacl), "{ty}");
    }
    // A wrong field is reported before the refusal, the earlier field first.
    for (ty, guid) in [("AL", 1336), ("OL", 1705)] {
        check(&[
            (&alloc::format!("S:({ty};QQ;GR;;;BA)"), Err(1004)),
            (&alloc::format!("S:({ty};SA;QQ;;;BA)"), Err(1336)),
            (&alloc::format!("S:({ty};SA;GR;;;QQ)"), Err(1337)),
            (&alloc::format!("S:({ty};SA;CR;12345;;BA)"), Err(guid)),
            (&alloc::format!("S:({ty};SA;CR;12345;;QQ)"), Err(guid)),
            (&alloc::format!("S:({ty};QQ;QQ;;;BA)"), Err(1004)),
            (&alloc::format!("S:({ty};SA;GR;;BA)"), Err(87)),
            (&alloc::format!("S:(AU;SA;GR;;;BA)({ty};SA;GR;;;BA)"), Err(87)),
            (&alloc::format!("O:BAS:({ty};SA;GR;;;BA)D:(A;;GR;;;BA)"), Err(87)),
        ]);
    }
    // OU without GUIDs: the plain audit type in a revision 4 list with four spare bytes.
    let b = ok("S:(OU;FA;GR;;;BA)");
    assert_eq!(hex(&b[20..]), "040024000100000002801800000000800102000000000005200000002002000000000000");
    let b = ok(&alloc::format!("S:(OU;SA;CR;{G1};{G2};SY)"));
    assert_eq!(
        hex(&b[20..]),
        "0400400001000000074038000001000003000000ffa4a86d520ed011a28600aa003049e2\
         b29557e45594d111aebd0000f80367c1010100000000000512000000"
    );
}

#[test]
fn parse_section_types() {
    let cond = "(@User.A == 1)";
    check(&[
        ("S:(A;;GA;;;WD)", Err(1804)),
        ("S:(D;QQ;GA;;;WD)", Err(1804)),
        ("S:(OA;;CR;12345;;WD)", Err(1804)),
        (&alloc::format!("S:(OD;;CR;{G1};;WD)"), Err(1804)),
        ("D:(AU;;GR;;;BA)", Err(1336)),
        ("D:(ML;;NW;;;LW)", Err(1804)),
        (&alloc::format!("S:(XA;;FR;;;BU;{cond})"), Err(1804)),
        (&alloc::format!("S:(XD;;FR;;;BU;{cond})"), Err(1804)),
        (&alloc::format!("D:(XU;SA;FR;;;BU;{cond})"), Err(1804)),
        (&alloc::format!("D:(XU;;FR;;;BU;{cond})"), Err(1804)),
        ("S:(XA;;FR;;;BU)", Err(1804)),
        ("D:(XU;SA;FR;;;BU)", Err(1804)),
        ("D:(RA;;;;;WD;(\"A\",TI,0,1))", Err(1804)),
        ("D:(SP;;;;;S-1-17-1)", Err(1804)),
        ("S:(XA;;FR;;;BU;(Member_of))", Err(1804)),
        ("D:(XU;SA;FR;;;BU;(Member_of))", Err(1804)),
    ]);
}

#[test]
fn parse_conditional() {
    // The sheet's example: header, mask, S-1-5-32-545, then the condition.
    let a = dace("XA;;FR;;;BU;(@User.Title == \"PM\")");
    assert_eq!(
        hex(&a.bytes),
        "09003800890012000102000000000005200000002102000061727478f90a0000005400690074006c00650010040000\
         0050004d0080000000"
    );
    check(&[
        ("D:(XA;;FR;;;BU;(Member_of {SID(SY)}))", Ok(80)),
        ("D:(XA;;FR;;;BU;(Member_of {SID(SY),SID(BA)}))", Ok(100)),
        ("D:(XA;;FR;;;BU;( Member_of  { SID(SY) , SID(BA) } ))", Ok(100)),
        ("D:(XA;;FR;;;BU;(member_of {sid(sy)}))", Ok(80)),
        ("D:(XA;;FR;;;BU;(Member_of SID(SY)))", Ok(76)),
        ("D:(XA;;FR;;;BU;(@User.Tags Not_Contains {1,2}))", Ok(100)),
        ("D:(XA;;FR;;;BU;(Not_Exists @Resource.Title))", Ok(72)),
        ("D:(XA;;FR;;;BU;((@User.A == 1) && (@User.B == 2 || @User.C == 3)))", Ok(116)),
        ("D:(XA;CIOI;FR;;;BU;(Member_of {SID(SY)}))(A;;GA;;;SY)", Ok(100)),
        ("D:(XD;;FR;;;BU;(@User.Level == 1))", Ok(84)),
        ("S:(XU;FA;FR;;;BU;(@User.Level == 1))", Ok(84)),
        ("D:(XA;;FR;;;BU;(1 == 1))", Ok(76)),
        // Refused conditions.
        ("D:(XA;;FR;;;BU;(Member_of))", Err(1336)),
        ("D:(XA;;FR;;;BU;())", Err(1336)),
        ("D:(XA;;FR;;;BU;( ))", Err(1336)),
        ("D:(XA;;FR;;;BU;(Member_of {}))", Err(1336)),
        ("D:(XA;;FR;;;BU;(Member_of {SID(ZZ)}))", Err(1336)),
        ("D:(XA;;FR;;;BU;(Member_of {SY}))", Err(1336)),
        ("D:(XA;;FR;;;BU;(Member_of {SID(SY)))", Err(1336)),
        ("D:(XA;;FR;;;BU;(Member_of {SID(SY)})", Err(1336)),
        ("D:(XA;;FR;;;BU;Member_of {SID(SY)})", Err(1336)),
        ("D:(XA;;FR;;;BU;(Nonsense {SID(SY)}))", Err(1336)),
        ("D:(XA;;FR;;;BU;(@User.Title ==))", Err(1336)),
        ("D:(XA;;FR;;;BU;(@User.Title = 1))", Err(1336)),
        ("D:(XA;;FR;;;BU;(@Nowhere.A == 1))", Err(1336)),
        ("D:(XA;;FR;;;BU;(@User.A == 1) extra)", Err(1336)),
        ("D:(XA;;FR;;;BU;(@User.A == 1);)", Err(1336)),
        ("D:(XA;;FR;;;BU;(@User.A == 1);x)", Err(1336)),
        ("D:(XA;;FR;;;BU;(\"a\" == \"a\"))", Err(1336)),
        ("D:(XA;;FR;;;BU;(Exists @User.Title))", Err(1336)),
        // No condition, or one where none belongs.
        ("D:(XA;;FR;;;BU)", Err(1336)),
        ("D:(XD;;FR;;;BU)", Err(1336)),
        ("S:(XU;SA;FR;;;BU)", Err(1336)),
        ("D:(XA;;FR;;;BU;)", Err(1336)),
        ("D:(ZA;;FR;;;BU)", Err(1336)),
        ("D:(A;;FR;;;BU;(Member_of {SID(SY)}))", Err(87)),
        ("D:(D;;FR;;;BU;(@User.A == 1))", Err(87)),
        ("S:(AU;SA;FR;;;BU;(@User.A == 1))", Err(87)),
        ("S:(ML;;NW;;;LW;(@User.A == 1))", Err(87)),
        ("D:(OA;;CR;;;BU;(@User.A == 1))", Err(87)),
    ]);
    // ZA: the callback type, four zero bytes more, a revision 4 list.
    let b = ok("D:(ZA;;FR;;;BU;(@User.A == 1))");
    assert_eq!(
        hex(&b[20..]),
        "04003c000100000009003400890012000102000000000005200000002102000061727478f902000000410004010000\
         00000000000302800000000000"
    );
}

/// The `CLAIM_SECURITY_ATTRIBUTE_RELATIVE_V1` of the first ACE of `S:(RA;...)`
/// and the size of that ACE.
fn ra_attribute(ace_text: &str) -> (Vec<u8>, usize) {
    let b = ok(&alloc::format!("S:({ace_text})"));
    let (_, _, s, _) = offsets(&b);
    let ace = s + 8;
    let size = usize::from(u16::from_le_bytes([b[ace + 2], b[ace + 3]]));
    // The header, the mask and the SID of Everyone come first.
    (b[ace + 20..ace + size].to_vec(), size)
}

/// `resource_attributes`: every value type starts right where the name ends
/// (sddl_parse_ra_names, sddl_parse_ra_multi); only the whole ACE is padded.
#[test]
fn resource_attribute_values_start_where_the_name_ends() {
    // An even number of characters ends at 2 modulo 4: no gap.
    for (name, at) in [("a", 24), ("ab", 26), ("abc", 28), ("abcd", 30), ("abcde", 32), ("abcdef", 34)] {
        let (st, _) = ra_attribute(&alloc::format!("RA;;;;;WD;(\"{name}\",TI,0,-1)"));
        assert_eq!(dword(&st, 16), at, "{name}");
        assert_eq!(&st[at..at + 8], &(-1i64).to_le_bytes());
    }
    // The structure of "ab" ends at 34; the ACE is padded: 20 + 36.
    assert_eq!(ra_attribute("RA;;;;;WD;(\"ab\",TI,0,-1)").1, 56);
    let (st, _) = ra_attribute("RA;;;;;WD;(\"ALongerAttributeName\",TI,0,-1,0,1)");
    assert_eq!((dword(&st, 16), dword(&st, 20), dword(&st, 24)), (0x46, 0x4e, 0x56));
    // The other types too: unsigned and boolean values, SIDs (a length and the bytes).
    let (st, _) = ra_attribute("RA;;;;;WD;(\"ab\",TU,0,7,8)");
    assert_eq!((dword(&st, 16), dword(&st, 20)), (30, 38));
    let (st, _) = ra_attribute("RA;;;;;WD;(\"ab\",TB,0,1,0)");
    assert_eq!((dword(&st, 16), dword(&st, 20)), (30, 38));
    let (st, _) = ra_attribute("RA;;;;;WD;(\"ab\",TD,0,SY,WD)");
    assert_eq!((dword(&st, 16), dword(&st, 20)), (30, 30 + 4 + 12));
    assert_eq!(dword(&st, 30), 12);
    // Strings and octets were never aligned.
    let (st, _) = ra_attribute("RA;;;;;WD;(\"ab\",TS,0,\"x\")");
    assert_eq!(dword(&st, 16), 26);
    let (st, _) = ra_attribute("RA;;;;;WD;(\"ab\",TX,0,0102)");
    assert_eq!((dword(&st, 16), dword(&st, 26)), (26, 2));
}

#[test]
fn parse_resource_attributes_and_policies() {
    let b = ok("S:(RA;;;;;WD;(\"N\",TI,0,-5))");
    assert_eq!(
        hex(&b[20..]),
        "02003c0001000000120034000000000001010000000000010000000014000000010000000000000001000000180000\
         004e000000fbffffffffffffff"
    );
    check(&[
        ("S:(RA;;;;;WD;(\"Dept\",TS,0,\"Lab\"))", Ok(88)),
        ("S:(RA;;;;;WD;(\"Dept\",TS,0,\"Lab\",\"Shop\",\"X\"))", Ok(108)),
        ("S:(RA;;;;;WD;(\"N\",TI,0,1,2,3))", Ok(104)),
        ("S:(RA;;;;;WD;(\"Who\",TD,0,SY,S-1-1-0))", Ok(112)),
        ("S:(RA;;;;;WD;(\"Bin\",TX,0,01,0203))", Ok(92)),
        ("S:(RA;CI;;;;WD;(\"N\",TI,0,5))", Ok(80)),
        ("S:(RA;;;;;WD; (\"N\" , TI , 0 , 5 ))", Ok(80)),
        ("S:(RA;;;;;WD;(\"A\",TI,0,1))(RA;;;;;WD;(\"B\",TS,0,\"z\"))", Ok(128)),
        ("S:(ML;;NW;;;LW)(RA;;;;;WD;(\"A\",TI,0,1))(AU;SA;FR;;;WD)", Ok(120)),
        ("S:(RA;;;;;WD;(\"N\",TX,0,0a0))", Ok(80)),
        ("S:(RA;;;;;WD;(\"\",TS,0,\"v\"))", Err(1336)),
        ("S:(RA;;;;;WD;(\"N\",TI,0xFFFFFFFF,5))", Err(1336)),
        ("S:(RA;;;;;BA;(\"N\",TI,0,5))", Err(1336)),
        ("S:(RA;;;;;WD)", Err(1336)),
        ("S:(RA;;;;;WD;)", Err(1336)),
        ("S:(RA;;;;;WD;())", Err(1336)),
        ("S:(RA;;;;;WD;(\"N\",TI,0))", Err(1336)),
        ("S:(RA;;;;;WD;(\"N\",TZ,0,5))", Err(1336)),
        ("S:(RA;;;;;WD;(N,TI,0,5))", Err(1336)),
        ("S:(RA;;;;;WD;(\"N\",TB,0,2))", Err(1336)),
        ("S:(RA;;;;;WD;(\"N\",TD,0,ZZ))", Err(1337)),
        ("S:(RA;;GA;;;WD;(\"N\",TI,0,5))", Err(1336)),
        ("S:(RA;SA;;;;WD;(\"N\",TI,0,5))", Err(1004)),
        ("S:(RA;;;;;WD;(\"N\",TI,0,5)", Err(1336)),
        // Scoped policies.
        ("S:(SP;;;;;S-1-17-22)", Ok(48)),
        ("S:(SP;;;;;S-1-17-1-2)", Ok(52)),
        ("S:(SP;CI;;;;S-1-17-22)", Ok(48)),
        ("S:(SP;;;;;WD)", Err(87)),
        ("S:(SP;;;;;S-1-5-18)", Err(87)),
        ("S:(SP;;GA;;;S-1-17-22)", Err(87)),
        ("S:(SP;;0x1;;;S-1-17-22)", Err(87)),
        ("S:(SP;;;;;S-1-17-22;(\"N\",TI,0,5))", Err(1336)),
        ("S:(SP;SA;;;;S-1-17-22)", Err(1004)),
    ]);
    assert_eq!(sace("SP;CI;;;;S-1-17-22").flags, 2);
    assert_eq!(sace("RA;OICIIO;;;;WD;(\"N\",TI,0,5)").flags, 0xb);
}

#[test]
fn parse_part_boundaries_and_white_space() {
    check(&[
        ("O:S-1-5-19G:S-1-5-20", Ok(44)),
        ("O:S-1-5-19D:(A;;FR;;;WD)", Ok(60)),
        ("O:S-1-5-19S:(ML;;NW;;;LW)", Ok(60)),
        ("G:S-1-5-32-545D:P", Ok(44)),
        ("G:S-1-5-32-545S:", Ok(44)),
        ("O:S-1-5-19D:", Ok(40)),
        ("O:S-1-5-0x13D:(A;;FR;;;WD)", Ok(60)),
        ("O:S-1-5-0xAD:(A;;FR;;;WD)", Ok(60)),
        ("O:S-1-5-19 D:(A;;FR;;;WD)", Err(1337)),
        ("O:S-1-5-19D", Err(1337)),
        ("O:S-1-5-19G", Err(1337)),
        ("O:S-1-5-19S", Err(1337)),
        ("O:S-1-5-19DG:SY", Err(1337)),
        ("O:S-1-5-19O:SY", Err(87)),
        ("D:(A;;FR;;;S-1-5-19)(A;;FW;;;S-1-5-20)S:(ML;;NW;;;S-1-16-4096)", Ok(96)),
        ("D:(A;;FR;;;S-1-5-19)O:S-1-5-20G:S-1-5-19", Ok(72)),
        ("O:LSG:NSD:S:", Ok(60)),
        ("G:BUO:BAS:D:", Ok(68)),
        ("O:SYG:BAD:PAI(A;;FR;;;WD)S:AI", Ok(84)),
        ("O:BA G:SY", Ok(48)),
    ]);
    let b = ok("O:S-1-5-0xAD:(A;;FR;;;WD)");
    let at = offsets(&b).0;
    assert_eq!(sddl::to_string(&Sid::from_bytes(&b[at..]).unwrap()), "S-1-5-10");
    // Spaces are free between and after parts; other white space is not a space.
    for (space, after_sid, after_ace, between_aces) in [
        (" ", Ok(32), Ok(48), Ok(72)),
        ("  ", Ok(32), Ok(48), Ok(72)),
        ("\t", Err(87), Ok(48), Err(1804)),
        ("\n", Err(87), Ok(48), Err(1804)),
        ("\r", Err(87), Ok(48), Err(1804)),
        ("\r\n", Err(87), Ok(48), Err(1804)),
    ] {
        let blank = space.trim_matches(' ').is_empty();
        let other = |n: usize| if blank { Ok(n) } else { Err(87) };
        check(&[
            (&alloc::format!("G:SY{space}"), after_sid),
            (&alloc::format!("D:(A;;FR;;;WD){space}"), after_ace),
            (&alloc::format!("S:(ML;;NW;;;LW){space}"), after_ace),
            (&alloc::format!("O:SYD:(A;;FR;;;WD)(A;;FW;;;BA){space}"), after_ace.map(|n| n + 36)),
            (&alloc::format!("D:(A;;FR;;;WD){space}(A;;FW;;;BA)"), between_aces),
            (&alloc::format!("D:{space}"), other(28)),
            (&alloc::format!("D:P{space}"), other(28)),
            (&alloc::format!("D:NO_ACCESS_CONTROL{space}"), other(20)),
            (&alloc::format!("G:SY{space}D:(A;;FR;;;WD){space}S:(ML;;NW;;;LW)"), other(88)),
            (&alloc::format!("{space}D:(A;;FR;;;WD)"), other(48)),
        ]);
    }
}

// ----- ConvertSecurityDescriptorToStringSecurityDescriptorW -------------------------------

#[test]
fn format_conditional_text() {
    for (a, b) in [
        ("D:(XA;;FR;;;BU;(@User.Title == \"PM\"))", "D:(XA;;FR;;;BU;(@USER.Title == \"PM\"))"),
        ("D:(XA;;FR;;;BU;(@User.A == 1 && @User.B == 2))", "D:(XA;;FR;;;BU;((@USER.A == 1) && (@USER.B == 2)))"),
        ("D:(XA;;FR;;;BU;(a || b && c))", "D:(XA;;FR;;;BU;((a) || ((b) && (c))))"),
        ("D:(XA;;FR;;;BU;(!(x)))", "D:(XA;;FR;;;BU;(!(x)))"),
        (
            "D:(XA;;FR;;;BU;(member_of_any{sid(S-1-5-32-544),SID(sy)}))",
            "D:(XA;;FR;;;BU;(Member_of_any {SID(BA), SID(SY)}))",
        ),
        ("D:(XA;CIOI;FR;;;BU;(@device.n >= 0x10))", "D:(XA;OICI;FR;;;BU;(@DEVICE.n >= 0x10))"),
        ("D:(XD;;FR;;;BU;(n > -3 || m < 010))", "D:(XD;;FR;;;BU;((n > -3) || (m < 010)))"),
        ("D:(XA;;FR;;;BU;(@Resource.b == #0102FF))", "D:(XA;;FR;;;BU;(@RESOURCE.b == #0102ff))"),
        ("S:(XU;SA;FR;;;BU;(Not_Exists @resource.t))", "S:(XU;SA;FR;;;BU;(Not_Exists @RESOURCE.t))"),
    ] {
        assert_eq!(round(a), b, "{a}");
        assert_eq!(round(b), b, "{b} again");
    }
    // The length: the text and one more per ACE.
    let one = "D:(XA;;FR;;;BU;(Member_of {SID(BA)}))";
    assert_eq!((one.len(), length(one)), (37, 38));
    let two = "D:(XA;;FR;;;BU;(Member_of {SID(BA)}))(XD;;FR;;;BU;(x == 1))";
    assert_eq!(length(two), two.len() + 2);
    // Resource attributes and scoped policies follow their own information bits.
    let sd = ok("S:(RA;;;;;WD;(\"Dept\",TS,0,\"Lab\",\"Shop\"))(SP;;;;;S-1-17-22)(RA;;;;;WD;(\"N\",TI,0x1,0x10))");
    assert_eq!(text(&sd, 0x1f).as_deref(), Ok("S:"));
    assert_eq!(
        text(&sd, 0x20).as_deref(),
        Ok("S:(RA;;;;;WD;(\"Dept\",TS,0x0,\"Lab\",\"Shop\"))(RA;;;;;WD;(\"N\",TI,0x1,16))")
    );
    assert_eq!(text(&sd, 0x40).as_deref(), Ok("S:(SP;;;;;S-1-17-22)"));
    let sd = ok("S:(RA;;;;;WD;(\"s\",TD,0,S-1-5-32-544,S-1-5-21-1-2-3-4))(RA;;;;;WD;(\"x\",TX,0,0102ff))");
    assert_eq!(
        text(&sd, 0x20).as_deref(),
        Ok("S:(RA;;;;;WD;(\"s\",TD,0x0,BA,S-1-5-21-1-2-3-4))(RA;;;;;WD;(\"x\",TX,0x0,0102ff))")
    );
}

const FULL: &str = "O:BAG:SYD:(A;;FA;;;WD)S:(AU;SAFA;FA;;;WD)(ML;;NW;;;LW)";

#[test]
fn format_selection() {
    let sd = ok(FULL);
    for (info, t) in [
        (1u32, "O:BA"),
        (2, "G:SY"),
        (4, "D:(A;;FA;;;WD)"),
        (8, "S:(AU;SAFA;FA;;;WD)"),
        (0x10, "S:(ML;;NW;;;LW)"),
        (0x18, "S:(AU;SAFA;FA;;;WD)(ML;;NW;;;LW)"),
        (3, "O:BAG:SY"),
        (5, "O:BAD:(A;;FA;;;WD)"),
        (9, "O:BAS:(AU;SAFA;FA;;;WD)"),
        (6, "G:SYD:(A;;FA;;;WD)"),
        (0xc, "D:(A;;FA;;;WD)S:(AU;SAFA;FA;;;WD)"),
        (7, "O:BAG:SYD:(A;;FA;;;WD)"),
        (0xf, "O:BAG:SYD:(A;;FA;;;WD)S:(AU;SAFA;FA;;;WD)"),
        (0x1f, FULL),
        (0x3f, FULL),
        (0x7f, FULL),
        (0xff, FULL),
        (0xffff_ffff, FULL),
        (0, ""),
        (0x8000_0000, ""),
        (0x2000_0000, ""),
        (0x4000_0000, ""),
        (0x1000_0000, ""),
        (0x10000, ""),
        (0x20, "S:"),
        (0x40, "S:"),
        (0x80, "S:"),
        (0x8000_0004, "D:(A;;FA;;;WD)"),
    ] {
        assert_eq!(text(&sd, info).as_deref(), Ok(t), "information {info:#x}");
    }
    let sd = ok("O:BA");
    for info in [2, 4, 8, 0x10] {
        assert_eq!(text(&sd, info).as_deref(), Ok(""), "owner only, {info:#x}");
    }
}

#[test]
fn format_ordering() {
    assert_eq!(round("G:SYO:BA"), "O:BAG:SY");
    assert_eq!(round("S:(AU;SAFA;FA;;;WD)D:(A;;FA;;;WD)G:SYO:BA"), "O:BAG:SYD:(A;;FA;;;WD)S:(AU;SAFA;FA;;;WD)");
    assert_eq!(round("D:(D;;GA;;;WD)(A;;GR;;;WD)"), "D:(D;;GA;;;WD)(A;;GR;;;WD)");
    assert_eq!(round("S:(ML;;NW;;;LW)(AU;SAFA;FA;;;WD)"), "S:(ML;;NW;;;LW)(AU;SAFA;FA;;;WD)");
}

#[test]
fn format_sid_text() {
    for (s, a) in [
        ("S-1-1-0", "WD"),
        ("S-1-3-0", "CO"),
        ("S-1-3-1", "CG"),
        ("S-1-3-4", "OW"),
        ("S-1-5-2", "NU"),
        ("S-1-5-4", "IU"),
        ("S-1-5-6", "SU"),
        ("S-1-5-7", "AN"),
        ("S-1-5-9", "ED"),
        ("S-1-5-10", "PS"),
        ("S-1-5-11", "AU"),
        ("S-1-5-12", "RC"),
        ("S-1-5-18", "SY"),
        ("S-1-5-19", "LS"),
        ("S-1-5-20", "NS"),
        ("S-1-5-32-544", "BA"),
        ("S-1-5-32-545", "BU"),
        ("S-1-5-32-546", "BG"),
        ("S-1-5-32-547", "PU"),
        ("S-1-5-32-548", "AO"),
        ("S-1-5-32-549", "SO"),
        ("S-1-5-32-550", "PO"),
        ("S-1-5-32-551", "BO"),
        ("S-1-5-32-552", "RE"),
        ("S-1-5-32-554", "RU"),
        ("S-1-5-32-555", "RD"),
        ("S-1-5-32-556", "NO"),
        ("S-1-5-32-558", "MU"),
        ("S-1-5-32-559", "LU"),
        ("S-1-5-32-568", "IS"),
        ("S-1-5-32-569", "CY"),
        ("S-1-5-32-573", "ER"),
        ("S-1-5-32-574", "CD"),
        ("S-1-5-32-575", "RA"),
        ("S-1-5-32-576", "ES"),
        ("S-1-5-32-577", "MS"),
        ("S-1-5-32-578", "HA"),
        ("S-1-5-32-579", "AA"),
        ("S-1-5-32-580", "RM"),
        ("S-1-5-33", "WR"),
        ("S-1-15-2-1", "AC"),
        ("S-1-16-4096", "LW"),
        ("S-1-16-8192", "ME"),
        ("S-1-16-8448", "MP"),
        ("S-1-16-12288", "HI"),
        ("S-1-16-16384", "SI"),
    ] {
        assert_eq!(round(&alloc::format!("O:{s}")), alloc::format!("O:{a}"), "{s}");
        assert_eq!(round(&alloc::format!("D:(A;;GA;;;{s})")), alloc::format!("D:(A;;GA;;;{a})"), "{s}");
    }
    let mut plain: Vec<String> = [
        "S-1-0-0",
        "S-1-2-0",
        "S-1-2-1",
        "S-1-3-2",
        "S-1-3-3",
        "S-1-5-1",
        "S-1-5-3",
        "S-1-5-8",
        "S-1-5-13",
        "S-1-5-14",
        "S-1-5-15",
        "S-1-5-17",
        "S-1-5-32-557",
        "S-1-5-32-560",
        "S-1-5-32-561",
        "S-1-5-32-562",
        "S-1-5-64-10",
        "S-1-5-80-0",
        "S-1-15-2-2",
        "S-1-15-3-1",
        "S-1-16-0",
        "S-1-16-4097",
        "S-1-16-20480",
        "S-1-16-28672",
        "S-1-5-99-1",
        "S-1-9-9",
    ]
    .iter()
    .map(|s| String::from(*s))
    .collect();
    for rid in [500, 501, 502, 512, 513, 514, 515, 516, 517, 518, 519, 520, 553, 571, 1000] {
        plain.push(alloc::format!("S-1-5-21-1-2-3-{rid}"));
    }
    for s in plain {
        assert_eq!(round(&alloc::format!("O:{s}")), alloc::format!("O:{s}"), "{s}");
    }
}

#[test]
fn format_rights_text() {
    let r = |m: u32| {
        let t = round(&alloc::format!("D:(A;;{m:#x};;;WD)"));
        String::from(t.strip_prefix("D:(A;;").unwrap().strip_suffix(";;;WD)").unwrap())
    };
    for (m, t) in [
        (0u32, ""),
        (1, "CC"),
        (2, "DC"),
        (4, "LC"),
        (8, "SW"),
        (0x10, "RP"),
        (0x20, "WP"),
        (0x40, "DT"),
        (0x80, "LO"),
        (0x100, "CR"),
        (0x10000, "SD"),
        (0x20000, "RC"),
        (0x40000, "WD"),
        (0x80000, "WO"),
        (0x1000_0000, "GA"),
        (0x2000_0000, "GX"),
        (0x4000_0000, "GW"),
        (0x8000_0000, "GR"),
        (0x1ff, "CCDCLCSWRPWPDTLOCR"),
        (0x101ff, "CCDCLCSWRPWPDTLOCRSD"),
        (0xf0000, "SDRCWDWO"),
        (0x1f01ff, "FA"),
        (0x120089, "FR"),
        (0x120116, "FW"),
        (0x1200a0, "FX"),
        (0x20019, "KR"),
        (0x20006, "KW"),
        (0xf003f, "KA"),
        (0x20039, "CCSWRPWPRC"),
        (0x2001d, "CCLCSWRPRC"),
        (0x3, "CCDC"),
        (0x7, "CCDCLC"),
        (0x7f, "CCDCLCSWRPWPDT"),
        (0x30000, "SDRC"),
        (0x60000, "RCWD"),
        (0xe0000, "RCWDWO"),
        (0xf000_0000, "GAGXGWGR"),
        (0xe000_0000, "GXGWGR"),
        (0x5000_0000, "GAGW"),
        (0x200, "0x200"),
        (0x400, "0x400"),
        (0x800, "0x800"),
        (0x1000, "0x1000"),
        (0x8000, "0x8000"),
        (0x10_0000, "0x100000"),
        (0x100_0000, "0x1000000"),
        (0x200_0000, "0x2000000"),
        (0x400_0000, "0x4000000"),
        (0x800_0000, "0x8000000"),
        (0x3ff, "0x3ff"),
        (0x1f_0000, "0x1f0000"),
        (0x12_0000, "0x120000"),
        (0x12019f, "0x12019f"),
        (0x120189, "0x120189"),
        (0x101f_01ff, "0x101f01ff"),
        (0xffff_ffff, "0xffffffff"),
        (0x7fff_ffff, "0x7fffffff"),
        (0x1234_5678, "0x12345678"),
    ] {
        assert_eq!(r(m), t, "{m:#x}");
    }
}

#[test]
fn format_canonical() {
    for (a, b) in [
        ("O:S-1-5-32-544", "O:BA"),
        ("D:(A;;0x10000000;;;WD)", "D:(A;;GA;;;WD)"),
        ("D:(A;;0x1f01ff;;;WD)", "D:(A;;FA;;;WD)"),
        ("D:(A;;GRGWGX;;;WD)", "D:(A;;GXGWGR;;;WD)"),
        ("D:(A;;RCSDWDWO;;;WD)", "D:(A;;SDRCWDWO;;;WD)"),
        ("D:(A;;WORCSDWD;;;WD)", "D:(A;;SDRCWDWO;;;WD)"),
        ("D:(A;;CRLO;;;WD)", "D:(A;;LOCR;;;WD)"),
        ("D:(A;;FRFW;;;WD)", "D:(A;;0x12019f;;;WD)"),
        ("D:(A;;KRKW;;;WD)", "D:(A;;CCDCLCSWRPRC;;;WD)"),
        ("D:(A;CIOI;GA;;;WD)", "D:(A;OICI;GA;;;WD)"),
        ("D:(A;IOCINP;GA;;;WD)", "D:(A;CINPIO;GA;;;WD)"),
        ("D:ARAIP(A;;GA;;;WD)", "D:PARAI(A;;GA;;;WD)"),
        ("D:AIP(A;;GA;;;WD)", "D:PAI(A;;GA;;;WD)"),
        ("S:PAI(AU;SA;GA;;;WD)", "S:PAI(AU;SA;GA;;;WD)"),
        ("S:(AU;FASA;GA;;;WD)", "S:(AU;SAFA;GA;;;WD)"),
        ("S:(AU;SAFAOICI;GA;;;WD)", "S:(AU;OICISAFA;GA;;;WD)"),
        ("S:(AU;IDSAFA;GA;;;WD)", "S:(AU;IDSAFA;GA;;;WD)"),
        ("S:(AU;IOSA;GA;;;WD)", "S:(AU;IOSA;GA;;;WD)"),
        ("S:(ML;;NXNRNW;;;LW)", "S:(ML;;NWNRNX;;;LW)"),
        ("S:(ML;;NW;;;S-1-16-4096)", "S:(ML;;NW;;;LW)"),
        ("S:(ML;;NW;;;S-1-16-12288)", "S:(ML;;NW;;;HI)"),
        ("S:(ML;;NW;;;S-1-16-4097)", "S:(ML;;NW;;;S-1-16-4097)"),
        ("D:NO_ACCESS_CONTROL", "D:NO_ACCESS_CONTROL"),
        ("D:PNO_ACCESS_CONTROL", "D:PNO_ACCESS_CONTROL"),
        ("D:", "D:"),
        ("D:PAI", "D:PAI"),
        ("S:NO_ACCESS_CONTROL", "S:NO_ACCESS_CONTROL"),
        ("S:", "S:"),
        ("", ""),
    ] {
        assert_eq!(round(a), b, "{a}");
    }
}

/// A descriptor with one raw ACE in its DACL or SACL.
fn raw(sacl: bool, ace: &[u8]) -> Vec<u8> {
    let size = (8 + ace.len()) as u16;
    let mut a = alloc::vec![2, 0];
    a.extend_from_slice(&size.to_le_bytes());
    a.extend_from_slice(&[1, 0, 0, 0]);
    a.extend_from_slice(ace);
    let mut p = Parts { revision: 1, ..Parts::default() };
    if sacl {
        p.control = crate::sd::SE_SACL_PRESENT;
        p.sacl = Some(a);
    } else {
        p.control = crate::sd::SE_DACL_PRESENT;
        p.dacl = Some(a);
    }
    p.to_relative()
}

/// A plain-layout ACE (type, flags, mask, S-1-1-0 or the given SID).
fn plain(kind: u8, flags: u8, mask: u32, sid: &Sid) -> Vec<u8> {
    let s = sid.to_bytes();
    let mut a = alloc::vec![kind, flags];
    a.extend_from_slice(&((8 + s.len()) as u16).to_le_bytes());
    a.extend_from_slice(&mask.to_le_bytes());
    a.extend_from_slice(&s);
    a
}

#[test]
fn format_ace_text() {
    let wd = crate::sid::world();
    let d = |kind: u8, flags: u8, mask: u32| text(&raw(false, &plain(kind, flags, mask, &wd)), 0xff);
    let s = |kind: u8, flags: u8, mask: u32, sid: &Sid| text(&raw(true, &plain(kind, flags, mask, sid)), 0xff);
    assert_eq!(d(0, 0, 1).as_deref(), Ok("D:(A;;CC;;;WD)"));
    assert_eq!(d(1, 0, 1).as_deref(), Ok("D:(D;;CC;;;WD)"));
    assert_eq!(d(0, 0xff, 1).as_deref(), Ok("D:(A;OICINPIOIDCR;CC;;;WD)"));
    assert_eq!(d(0, 0xc0, 1).as_deref(), Ok("D:(A;;CC;;;WD)"));
    assert_eq!(d(9, 0, 1).as_deref(), Ok("D:(XA;;CC;;;WD)"));
    assert_eq!(d(10, 0, 1).as_deref(), Ok("D:(XD;;CC;;;WD)"));
    for (f, t) in [(0x40, "SA"), (0x80, "FA"), (0xc0, "SAFA"), (0, "")] {
        assert_eq!(s(2, f, 1, &wd), Ok(alloc::format!("S:(AU;{t};CC;;;WD)")));
        assert_eq!(s(3, f, 1, &wd), Ok(alloc::format!("S:(AL;{t};CC;;;WD)")));
    }
    for (m, t) in [
        (0, ""),
        (1, "NW"),
        (2, "NR"),
        (4, "NX"),
        (3, "NWNR"),
        (5, "NWNX"),
        (6, "NRNX"),
        (7, "NWNRNX"),
        (8, "0x8"),
        (0xf, "0xf"),
    ] {
        assert_eq!(s(0x11, 0, m, &crate::sid::mandatory(0x1000)), Ok(alloc::format!("S:(ML;;{t};;;LW)")), "{m}");
    }
    for (rid, t) in [
        (0x1000, "LW"),
        (0x2000, "ME"),
        (0x2100, "MP"),
        (0x3000, "HI"),
        (0x4000, "SI"),
        (0, "S-1-16-0"),
        (4095, "S-1-16-4095"),
    ] {
        assert_eq!(s(0x11, 0, 1, &crate::sid::mandatory(rid)), Ok(alloc::format!("S:(ML;;NW;;;{t})")));
    }
    for (f, t) in [(2, "CI"), (3, "OICI"), (8, "IO"), (0x10, "ID"), (0xc0, "")] {
        assert_eq!(s(0x11, f, 1, &crate::sid::mandatory(0x1000)), Ok(alloc::format!("S:(ML;{t};NW;;;LW)")));
    }
    assert_eq!(s(0x11, 0, 1, &wd).as_deref(), Ok("S:(ML;;NW;;;WD)"));
    assert_eq!(s(0x11, 0, 1, &crate::sid::builtin(544)).as_deref(), Ok("S:(ML;;NW;;;BA)"));
    // The types that have no text, and the SACL's types in a DACL.
    for kind in [4u8, 7, 8, 0xc, 0xd, 0xe, 0xf, 0x10, 0x12, 0x13, 0x14, 0x15, 0x16, 0x20, 0x80, 0xff, 2, 3, 0x11] {
        assert_eq!(d(kind, 0, 1), Err(1336), "type {kind:#x}");
    }
    // An object ACE whose body is not one: the flags word says a GUID
    // follows, the ACE ends before a SID.
    for kind in [5u8, 6] {
        assert_eq!(d(kind, 0, 1), Err(1337), "type {kind}");
    }
    assert_eq!(d(0xb, 0, 1), Err(805));
}

#[test]
fn format_length() {
    for (s, n) in [
        ("O:BA", 4),
        ("O:BAG:SY", 8),
        ("D:", 2),
        ("", 0),
        ("O:BAD:", 6),
        ("S:", 2),
        ("D:(A;;GA;;;WD)", 15),
        ("D:NO_ACCESS_CONTROL", 20),
        ("S:(AU;SAFA;GA;;;WD)", 20),
        ("D:(A;;GA;;;WD)(D;;GW;;;BA)(A;;GR;;;SY)", 41),
        ("D:(A;;FA;;;WD)S:(AU;SAFA;FA;;;WD)", 35),
        (FULL, 57),
        ("D:(A;;0x12019f;;;WD)", 23),
        ("D:(A;;CCDCLCSWRPRC;;;WD)", 25),
    ] {
        assert_eq!(length(s), n, "{s}");
    }
}

#[test]
fn format_errors() {
    // An invalid owner SID.
    for (rev, count) in [(2u8, 1u8), (1, 16), (0, 1)] {
        let mut sid = alloc::vec![rev, count, 0, 0, 0, 0, 0, 5];
        sid.resize(8 + 4 * usize::from(count), 0);
        let p = Parts { revision: 1, owner: Some(sid), ..Parts::default() };
        assert_eq!(format(&p, 1).map_err(SecErr::win32), Err(1337), "revision {rev}, count {count}");
    }
}
