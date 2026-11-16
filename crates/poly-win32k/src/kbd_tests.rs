//! The German layout against the visible tests' recorded results
//! (`specs/windows/user32.dll/{MapVirtualKeyW,ToUnicode,SendInput}.tests`)
//! and the spec sheets' rules.

extern crate std;

use super::*;
use std::format;
use std::string::String;
use std::vec::Vec;

/// `mvk_scan`'s recorded output.
const MVK_SCAN: &str = "0x00: 00/00 1b/1b 31/31 32/32 33/33 34/34 35/35 36/36
0x08: 37/37 38/38 39/39 30/30 db/db dd/dd 08/08 09/09
0x10: 51/51 57/57 45/45 52/52 54/54 5a/5a 55/55 49/49
0x18: 4f/4f 50/50 ba/ba bb/bb 0d/0d 11/a2 41/41 53/53
0x20: 44/44 46/46 47/47 48/48 4a/4a 4b/4b 4c/4c c0/c0
0x28: de/de dc/dc 10/a0 bf/bf 59/59 58/58 43/43 56/56
0x30: 42/42 4e/4e 4d/4d bc/bc be/be bd/bd 10/a1 6a/6a
0x38: 12/a4 20/20 14/14 70/70 71/71 72/72 73/73 74/74
0x40: 75/75 76/76 77/77 78/78 79/79 90/90 91/91 24/24
0x48: 26/26 21/21 6d/6d 25/25 0c/0c 27/27 6b/6b 23/23
0x50: 28/28 22/22 2d/2d 2e/2e 2c/2c 00/00 e2/e2 7a/7a
0x58: 7b/7b 0c/0c ee/ee f1/f1 ea/ea f9/f9 f5/f5 f3/f3
0x60: 00/00 00/00 fb/fb 2f/2f 7c/7c 7d/7d 7e/7e 7f/7f
0x68: 80/80 81/81 82/82 83/83 84/84 85/85 86/86 ed/ed
0x70: 00/00 e9/e9 00/00 c1/c1 00/00 00/00 87/87 00/00
0x78: 00/00 00/00 00/00 eb/eb 09/09 00/00 c2/c2 00/00
0xe01c: vk=0x0d vk_ex=0x0d
0xe01d: vk=0x11 vk_ex=0xa3
0xe035: vk=0x6f vk_ex=0x6f
0xe037: vk=0x2c vk_ex=0x2c
0xe038: vk=0x12 vk_ex=0xa5
0xe047: vk=0x24 vk_ex=0x24
0xe048: vk=0x26 vk_ex=0x26
0xe049: vk=0x21 vk_ex=0x21
0xe04b: vk=0x25 vk_ex=0x25
0xe04d: vk=0x27 vk_ex=0x27
0xe04f: vk=0x23 vk_ex=0x23
0xe050: vk=0x28 vk_ex=0x28
0xe051: vk=0x22 vk_ex=0x22
0xe052: vk=0x2d vk_ex=0x2d
0xe053: vk=0x2e vk_ex=0x2e
0xe05b: vk=0x5b vk_ex=0x5b
0xe05c: vk=0x5c vk_ex=0x5c
0xe05d: vk=0x5d vk_ex=0x5d
0xe11d: vk=0x13 vk_ex=0x13
0xe02a: vk=0x00 vk_ex=0x00
0xe046: vk=0x03 vk_ex=0x03
0xe0ff: vk=0x00 vk_ex=0x00
";

#[test]
fn mvk_scan_matches_the_oracle() {
    let mut out = String::new();
    for row in (0..0x80u32).step_by(8) {
        out += &format!("{row:#04x}:");
        for sc in row..row + 8 {
            out += &format!(" {:02x}/{:02x}", map_virtual_key(sc, 1), map_virtual_key(sc, 3));
        }
        out += "\n";
    }
    for e in [
        0xE01Cu32, 0xE01D, 0xE035, 0xE037, 0xE038, 0xE047, 0xE048, 0xE049, 0xE04B, 0xE04D, 0xE04F, 0xE050, 0xE051,
        0xE052, 0xE053, 0xE05B, 0xE05C, 0xE05D, 0xE11D, 0xE02A, 0xE046, 0xE0FF,
    ] {
        out += &format!("{e:#06x}: vk={:#04x} vk_ex={:#04x}\n", map_virtual_key(e, 1), map_virtual_key(e, 3));
    }
    assert_eq!(out, MVK_SCAN);
}

#[test]
fn mvk_types_matches_the_oracle() {
    for map in [5u32, 6, 7, 8, 0x100, 0xFFFF_FFFF] {
        assert_eq!(try_map_virtual_key(0x41, map), Err(87));
        assert_eq!(try_map_virtual_key(0x1E, map), Err(87));
    }
    for vk in [0x100u32, 0x141, 0xFFFF] {
        assert_eq!(try_map_virtual_key(vk, 0), Err(87));
        assert_eq!(try_map_virtual_key(vk, 2), Err(87));
    }
    assert_eq!(try_map_virtual_key(0x100, 1), Ok(0));
    assert_eq!(try_map_virtual_key(0x1E1E, 1), Ok(0));
    // vk, vsc, vk, vk_ex, vsc_ex, vk_ex
    let rows = [
        (0xA0u32, 0x2Au32, 0x10u32, 0xA0u32, 0x2Au32, 0xA0u32),
        (0xA1, 0x36, 0x10, 0xA1, 0x36, 0xA1),
        (0xA2, 0x1D, 0x11, 0xA2, 0x1D, 0xA2),
        (0xA3, 0x1D, 0x11, 0xA2, 0xE01D, 0xA3),
        (0xA4, 0x38, 0x12, 0xA4, 0x38, 0xA4),
        (0xA5, 0x38, 0x12, 0xA4, 0xE038, 0xA5),
        (0x25, 0x4B, 0x25, 0x25, 0x4B, 0x25),
        (0x26, 0x48, 0x26, 0x26, 0x48, 0x26),
        (0x2E, 0x53, 0x2E, 0x2E, 0x53, 0x2E),
    ];
    for (vk, s, v, vx, sx, vxx) in rows {
        assert_eq!(map_virtual_key(vk, 0), s, "vk {vk:#x}");
        assert_eq!(map_virtual_key(s, 1), v);
        assert_eq!(map_virtual_key(s, 3), vx);
        assert_eq!(map_virtual_key(vk, 4), sx);
        assert_eq!(map_virtual_key(sx, 3), vxx);
    }
    assert_eq!(HKL, 0x0407_2000);
}

#[test]
fn vk_to_vsc_per_the_sheet() {
    let rows: &[(u32, u32)] = &[
        (0x41, 0x1E),
        (0x42, 0x30),
        (0x45, 0x12),
        (0x4D, 0x32),
        (0x51, 0x10),
        (0x59, 0x2C),
        (0x5A, 0x15),
        (0x30, 0x0B),
        (0x31, 0x02),
        (0x32, 0x03),
        (0x36, 0x07),
        (0x39, 0x0A),
        (0xBA, 0x1A),
        (0xBB, 0x1B),
        (0xBC, 0x33),
        (0xBD, 0x35),
        (0xBE, 0x34),
        (0xBF, 0x2B),
        (0xC0, 0x27),
        (0xDB, 0x0C),
        (0xDC, 0x29),
        (0xDD, 0x0D),
        (0xDE, 0x28),
        (0xE2, 0x56),
        (0x60, 0x52),
        (0x65, 0x4C),
        (0x6A, 0x37),
        (0x6B, 0x4E),
        (0x6D, 0x4A),
        (0x6E, 0x53),
        (0x6F, 0x35),
        (0x90, 0x45),
        (0x25, 0x4B),
        (0x26, 0x48),
        (0x27, 0x4D),
        (0x28, 0x50),
        (0x2D, 0x52),
        (0x2E, 0x53),
        (0x24, 0x47),
        (0x23, 0x4F),
        (0x21, 0x49),
        (0x22, 0x51),
        (0x10, 0x2A),
        (0xA0, 0x2A),
        (0xA1, 0x36),
        (0x11, 0x1D),
        (0xA2, 0x1D),
        (0xA3, 0x1D),
        (0x12, 0x38),
        (0xA4, 0x38),
        (0xA5, 0x38),
        (0x0D, 0x1C),
        (0x20, 0x39),
        (0x08, 0x0E),
        (0x09, 0x0F),
        (0x1B, 0x01),
        (0x70, 0x3B),
        (0x79, 0x44),
        (0x7B, 0x58),
        (0x87, 0x76),
        (0x14, 0x3A),
        (0x91, 0x46),
        (0x5B, 0x5B),
        (0x5C, 0x5C),
        (0x5D, 0x5D),
        (0x2C, 0x54),
        (0xE7, 0),
        (0, 0),
        (0xFF, 0),
        (0x13, 0),
        (0x03, 0x46),
    ];
    for &(vk, sc) in rows {
        assert_eq!(map_virtual_key(vk, 0), sc, "vk {vk:#x}");
    }
    let ex: &[(u32, u32)] = &[
        (0x6F, 0xE035),
        (0xA3, 0xE01D),
        (0xA5, 0xE038),
        (0x5B, 0xE05B),
        (0x5C, 0xE05C),
        (0x5D, 0xE05D),
        (0x03, 0xE046),
        (0x13, 0xE11D),
        (0x25, 0x4B),
        (0x2D, 0x52),
        (0x2C, 0x54),
        (0x41, 0x1E),
    ];
    for &(vk, sc) in ex {
        assert_eq!(map_virtual_key(vk, 4), sc, "vk {vk:#x}");
    }
}

#[test]
fn vk_to_char_per_the_sheet() {
    let rows: &[(u32, u32)] = &[
        (0x41, 0x41),
        (0x5A, 0x5A),
        (0x30, 0x30),
        (0x39, 0x39),
        (0xBA, 0xFC),
        (0xBB, 0x2B),
        (0xBC, 0x2C),
        (0xBD, 0x2D),
        (0xBE, 0x2E),
        (0xBF, 0x23),
        (0xC0, 0xF6),
        (0xDB, 0xDF),
        (0xDE, 0xE4),
        (0xE2, 0x3C),
        (0xDC, 0x8000_005E),
        (0xDD, 0x8000_00B4),
        (0x60, 0x30),
        (0x65, 0x35),
        (0x6A, 0x2A),
        (0x6B, 0x2B),
        (0x6D, 0x2D),
        (0x6E, 0x2C),
        (0x6F, 0x2F),
        (0x0D, 0x0D),
        (0x20, 0x20),
        (0x08, 0x08),
        (0x09, 0x09),
        (0x1B, 0x1B),
        (0x03, 0x03),
        (0xFF, 0x5E),
        (0x10, 0),
        (0xA0, 0),
        (0x25, 0),
        (0x70, 0),
        (0x14, 0),
        (0x90, 0),
        (0x5B, 0),
        (0xE7, 0),
    ];
    for &(vk, c) in rows {
        assert_eq!(map_virtual_key(vk, 2), c, "vk {vk:#x}");
    }
}

/// `ToUnicode` as `user32` exports it, over an 8-unit buffer prefilled
/// with 0xAAAA.
fn tu(vk: u32, scan: u32, ks: &[u8; 256], n: usize, flags: u32, dead: &mut DeadState) -> (i32, [u16; 4]) {
    let mut out = [0xAAAAu16; 8];
    let r = to_unicode(vk, scan, ks, &mut out[..n], flags, dead);
    if n == 0 {
        out[0] = 0;
    }
    (r, [out[0], out[1], out[2], out[3]])
}

#[test]
fn tu_buffers_matches_the_oracle() {
    let expect = "a, size 8: 1, buffer 0061 0000 0000 0000
a, size 1: 1, buffer 0061 aaaa aaaa aaaa
a, size 0: 0, buffer 0000 aaaa aaaa aaaa
a key-up, size 8: 0, buffer 0000 0000 0000 0000
^, size 8: -1, buffer 005e 0000 0000 0000
then x, size 1: 1, buffer 005e aaaa aaaa aaaa
then x again, size 8: 1, buffer 0078 0000 0000 0000
^, size 0: 0, buffer 0000 aaaa aaaa aaaa
then x, size 8: 1, buffer 0078 0000 0000 0000
vk 0: 0, buffer 0000 0000 0000 0000
vk 0xff: 0, buffer 0000 0000 0000 0000
a with scan 0: 1, buffer 0061 0000 0000 0000
Ctrl+ue key (0xba): 1, buffer 001b 0000 0000 0000
Ctrl+Space: 1, buffer 0020 0000 0000 0000
Ctrl+2: 0, buffer 0000 0000 0000 0000
Ctrl+Back: 1, buffer 007f 0000 0000 0000
Ctrl+Enter: 1, buffer 000a 0000 0000 0000
final a: 1, buffer 0061 0000 0000 0000
";
    let s = [0u8; 256];
    let mut c = [0u8; 256];
    c[0x11] = 0x80;
    let calls: &[(&str, u32, u32, bool, usize)] = &[
        ("a, size 8", 0x41, 0x1E, false, 8),
        ("a, size 1", 0x41, 0x1E, false, 1),
        ("a, size 0", 0x41, 0x1E, false, 0),
        ("a key-up, size 8", 0x41, 0x1E | 0x8000, false, 8),
        ("^, size 8", 0xDC, 0x29, false, 8),
        ("then x, size 1", 0x58, 0x2D, false, 1),
        ("then x again, size 8", 0x58, 0x2D, false, 8),
        ("^, size 0", 0xDC, 0x29, false, 0),
        ("then x, size 8", 0x58, 0x2D, false, 8),
        ("vk 0", 0, 0, false, 8),
        ("vk 0xff", 0xFF, 0, false, 8),
        ("a with scan 0", 0x41, 0, false, 8),
        ("Ctrl+ue key (0xba)", 0xBA, 0x1A, true, 8),
        ("Ctrl+Space", 0x20, 0x39, true, 8),
        ("Ctrl+2", 0x32, 0x03, true, 8),
        ("Ctrl+Back", 0x08, 0x0E, true, 8),
        ("Ctrl+Enter", 0x0D, 0x1C, true, 8),
        ("final a", 0x41, 0x1E, false, 8),
    ];
    let mut dead = DeadState::default();
    let mut out = String::new();
    for &(label, vk, scan, ctrl, n) in calls {
        let (r, b) = tu(vk, scan, if ctrl { &c } else { &s }, n, 0, &mut dead);
        out += &format!("{label}: {r}, buffer {:04x} {:04x} {:04x} {:04x}\n", b[0], b[1], b[2], b[3]);
    }
    assert_eq!(out, expect);
}

#[test]
fn tu_dead_matches_the_oracle() {
    let expect = "^: -1 005e
a: 1 00e2
a again: 1 0061
^: -1 005e
Space: 1 005e
^: -1 005e
x: 2 005e 0078
^: -1 005e
^: 2 005e 005e
a: 1 0061
acute: -1 00b4
e: 1 00e9
acute: -1 00b4
E: 1 00c9
grave: -1 0060
A: 1 00c0
^ (flags 4): -1 005e
a: 1 0061
^: -1 005e
^ up: 0
a: 1 00e2
^: -1 005e
VK_SHIFT: 0
a: 1 00e2
final a: 1 0061
";
    let (circ, acute, a, e, x, space, shift) = (0xDCu32, 0xDDu32, 0x41u32, 0x45u32, 0x58u32, 0x20u32, 0x10u32);
    let seq: &[(&str, u32, bool, bool, u32)] = &[
        ("^", circ, false, false, 0),
        ("a", a, false, false, 0),
        ("a again", a, false, false, 0),
        ("^", circ, false, false, 0),
        ("Space", space, false, false, 0),
        ("^", circ, false, false, 0),
        ("x", x, false, false, 0),
        ("^", circ, false, false, 0),
        ("^", circ, false, false, 0),
        ("a", a, false, false, 0),
        ("acute", acute, false, false, 0),
        ("e", e, false, false, 0),
        ("acute", acute, false, false, 0),
        ("E", e, false, true, 0),
        ("grave", acute, false, true, 0),
        ("A", a, false, true, 0),
        ("^ (flags 4)", circ, false, false, 4),
        ("a", a, false, false, 0),
        ("^", circ, false, false, 0),
        ("^ up", circ, true, false, 0),
        ("a", a, false, false, 0),
        ("^", circ, false, false, 0),
        ("VK_SHIFT", shift, false, true, 0),
        ("a", a, false, false, 0),
        ("final a", a, false, false, 0),
    ];
    let mut dead = DeadState::default();
    let mut out = String::new();
    for &(label, vk, up, sh, flags) in seq {
        let scan = map_virtual_key(vk, 0) | if up { 0x8000 } else { 0 };
        let mut ks = [0u8; 256];
        if sh {
            ks[0x10] = 0x80;
        }
        let mut buf = [0u16; 8];
        let r = to_unicode(vk, scan, &ks, &mut buf, flags, &mut dead);
        let n = if r < 0 { 1 } else { r as usize };
        let units: Vec<String> = buf[..n].iter().map(|u| format!(" {u:04x}")).collect();
        out += &format!("{label}: {r}{}\n", units.concat());
    }
    assert_eq!(out, expect);
}

/// One `ToUnicode` with the given modifiers; `None` for 0 units.
fn ch(vk: u32, shift: bool, caps: bool, ctrl: bool, alt: bool) -> Option<u16> {
    let mut ks = [0u8; 256];
    ks[0x10] = if shift { 0x80 } else { 0 };
    ks[0x11] = if ctrl { 0x80 } else { 0 };
    ks[0x12] = if alt { 0x80 } else { 0 };
    ks[0x14] = u8::from(caps);
    let mut buf = [0u16; 4];
    let mut dead = DeadState::default();
    match to_unicode(vk, 0, &ks, &mut buf, 0, &mut dead) {
        1 => Some(buf[0]),
        0 => None,
        r => panic!("vk {vk:#x}: {r}"),
    }
}

/// The eight states of `ToUnicode.yaml`: none, Shift, Caps, Shift+Caps,
/// Ctrl, Ctrl+Shift, AltGr, Alt.
fn states(vk: u32) -> [Option<u16>; 8] {
    [
        ch(vk, false, false, false, false),
        ch(vk, true, false, false, false),
        ch(vk, false, true, false, false),
        ch(vk, true, true, false, false),
        ch(vk, false, false, true, false),
        ch(vk, true, false, true, false),
        ch(vk, false, false, true, true),
        ch(vk, false, false, false, true),
    ]
}

#[test]
fn character_states_per_the_sheet() {
    const N: Option<u16> = None;
    let s = |c: u16| Some(c);
    let a = |c: char| Some(c as u16);
    let rows: &[(u32, [Option<u16>; 8])] = &[
        (0x41, [a('a'), a('A'), a('A'), a('a'), s(1), s(1), N, a('a')]),
        (0x51, [a('q'), a('Q'), a('Q'), a('q'), s(0x11), s(0x11), a('@'), a('q')]),
        (0x45, [a('e'), a('E'), a('E'), a('e'), s(5), s(5), s(0x20AC), a('e')]),
        (0x4D, [a('m'), a('M'), a('M'), a('m'), s(0x0D), s(0x0D), s(0xB5), a('m')]),
        (0x59, [a('y'), a('Y'), a('Y'), a('y'), s(0x19), s(0x19), N, a('y')]),
        (0x5A, [a('z'), a('Z'), a('Z'), a('z'), s(0x1A), s(0x1A), N, a('z')]),
        (0x31, [a('1'), a('!'), a('!'), a('1'), N, N, N, a('1')]),
        (0x32, [a('2'), a('"'), a('"'), a('2'), N, s(0), s(0xB2), a('2')]),
        (0x33, [a('3'), s(0xA7), s(0xA7), a('3'), N, N, s(0xB3), a('3')]),
        (0x37, [a('7'), a('/'), a('/'), a('7'), N, N, a('{'), a('7')]),
        (0x38, [a('8'), a('('), a('('), a('8'), N, N, a('['), a('8')]),
        (0x39, [a('9'), a(')'), a(')'), a('9'), N, N, a(']'), a('9')]),
        (0x30, [a('0'), a('='), a('='), a('0'), N, N, a('}'), a('0')]),
        (0xDB, [s(0xDF), a('?'), a('?'), s(0xDF), N, N, a('\\'), s(0xDF)]),
        (0xBA, [s(0xFC), s(0xDC), s(0xDC), s(0xFC), s(0x1B), N, N, s(0xFC)]),
        (0xBB, [a('+'), a('*'), a('*'), a('+'), s(0x1D), N, a('~'), a('+')]),
        (0xBF, [a('#'), a('\''), a('\''), a('#'), s(0x1C), N, N, a('#')]),
        (0xC0, [s(0xF6), s(0xD6), s(0xD6), s(0xF6), N, N, N, s(0xF6)]),
        (0xDE, [s(0xE4), s(0xC4), s(0xC4), s(0xE4), N, N, N, s(0xE4)]),
        (0xE2, [a('<'), a('>'), a('<'), a('>'), N, N, a('|'), a('<')]),
        (0xBC, [a(','), a(';'), a(';'), a(','), N, N, N, a(',')]),
        (0xBD, [a('-'), a('_'), a('-'), a('_'), N, s(0x1F), N, a('-')]),
        (0x20, [s(0x20), s(0x20), s(0x20), s(0x20), s(0x20), N, N, s(0x20)]),
        (0x0D, [s(0x0D), s(0x0D), s(0x0D), s(0x0D), s(0x0A), N, N, s(0x0D)]),
        (0x08, [s(8), s(8), s(8), s(8), s(0x7F), N, N, s(8)]),
        (0x09, [s(9), s(9), s(9), s(9), N, N, N, s(9)]),
        (0x1B, [s(0x1B), s(0x1B), s(0x1B), s(0x1B), s(0x1B), N, N, s(0x1B)]),
        (0x65, [a('5'), N, a('5'), N, N, N, N, N]),
        (0x6E, [a(','), a(','), a(','), a(','), N, N, N, a(',')]),
        (0x25, [N; 8]),
    ];
    for (vk, want) in rows {
        assert_eq!(&states(*vk), want, "vk {vk:#x}");
    }
}

/// `SendInput.yaml`'s typing and dead keys through `translate_key`.
#[test]
fn translate_key_per_si_typing_and_si_deadkeys() {
    let none = [0u8; 256];
    let mut shift = [0u8; 256];
    shift[0x10] = 0x80;
    shift[0xA0] = 0x80;
    let mut ctrl = [0u8; 256];
    ctrl[0x11] = 0x80;
    ctrl[0xA2] = 0x80;
    let mut dead = DeadState::default();
    let one = |vk: u64, l: u64, ks: &[u8; 256], dead: &mut DeadState| {
        let m: Vec<CharMsg> = translate_key(WM_KEYDOWN, vk, l, ks, dead).iter().collect();
        m.iter().map(|m| (m.msg, m.wparam, m.lparam)).collect::<Vec<_>>()
    };
    assert_eq!(one(0x41, 0x001E_0001, &none, &mut dead), [(WM_CHAR, 0x61, 0x001E_0001)]);
    assert_eq!(one(0x41, 0x001E_0001, &shift, &mut dead), [(WM_CHAR, 0x41, 0x001E_0001)]);
    assert_eq!(one(0x10, 0x002A_0001, &shift, &mut dead), []);
    assert_eq!(one(0x31, 0x0002_0001, &none, &mut dead), [(WM_CHAR, 0x31, 0x0002_0001)]);
    assert_eq!(one(0x0D, 0x001C_0001, &none, &mut dead), [(WM_CHAR, 0x0D, 0x001C_0001)]);
    assert_eq!(one(0x08, 0x000E_0001, &none, &mut dead), [(WM_CHAR, 0x08, 0x000E_0001)]);
    assert_eq!(one(0x09, 0x000F_0001, &none, &mut dead), [(WM_CHAR, 0x09, 0x000F_0001)]);
    assert_eq!(one(0x1B, 0x0001_0001, &none, &mut dead), [(WM_CHAR, 0x1B, 0x0001_0001)]);
    assert_eq!(one(0x25, 0x014B_0001, &none, &mut dead), []);
    assert_eq!(one(0x2E, 0x0153_0001, &none, &mut dead), []);
    assert_eq!(one(0x41, 0x001E_0001, &ctrl, &mut dead), [(WM_CHAR, 0x01, 0x001E_0001)]);
    assert_eq!(one(0x5A, 0x0015_0001, &none, &mut dead), [(WM_CHAR, 0x7A, 0x0015_0001)]);
    assert_eq!(one(0x59, 0x002C_0001, &none, &mut dead), [(WM_CHAR, 0x79, 0x002C_0001)]);
    assert_eq!(one(0xBA, 0x001A_0001, &none, &mut dead), [(WM_CHAR, 0xFC, 0x001A_0001)]);
    assert_eq!(one(0xDB, 0x000C_0001, &none, &mut dead), [(WM_CHAR, 0xDF, 0x000C_0001)]);
    assert_eq!(one(0x42, 0x0030_0002, &none, &mut dead), [(WM_CHAR, 0x62, 0x0030_0002)]);
    assert_eq!(one(0x43, 0x402E_0001, &none, &mut dead), [(WM_CHAR, 0x63, 0x402E_0001)]);
    assert_eq!(one(0x41, 0x0000_0001, &none, &mut dead), [(WM_CHAR, 0x61, 0x0000_0001)]);
    // si_deadkeys.
    assert_eq!(one(0xDC, 0x0029_0001, &none, &mut dead), [(WM_DEADCHAR, 0x5E, 0x0029_0001)]);
    assert_eq!(one(0x41, 0x001E_0001, &none, &mut dead), [(WM_CHAR, 0xE2, 0x001E_0001)]);
    assert_eq!(one(0xDD, 0x000D_0001, &none, &mut dead), [(WM_DEADCHAR, 0xB4, 0x000D_0001)]);
    assert_eq!(one(0x45, 0x0012_0001, &none, &mut dead), [(WM_CHAR, 0xE9, 0x0012_0001)]);
    assert_eq!(one(0xDC, 0x0029_0001, &none, &mut dead), [(WM_DEADCHAR, 0x5E, 0x0029_0001)]);
    assert_eq!(one(0x20, 0x0039_0001, &none, &mut dead), [(WM_CHAR, 0x5E, 0x0039_0001)]);
    assert_eq!(one(0xDC, 0x0029_0001, &none, &mut dead), [(WM_DEADCHAR, 0x5E, 0x0029_0001)]);
    assert_eq!(dead.pending(), Some(0x5E));
    assert_eq!(one(0x58, 0x002D_0001, &none, &mut dead), [(WM_CHAR, 0x5E, 0x022D_0001), (WM_CHAR, 0x78, 0x002D_0001)]);
    assert_eq!(dead.pending(), None);
    // VK_PACKET's unit is the input path's; key-ups and other messages post nothing.
    assert!(translate_key(WM_KEYDOWN, u64::from(VK_PACKET), 1, &none, &mut dead).is_empty());
    assert!(translate_key(0x0101, 0x41, 0xC01E_0001, &none, &mut dead).is_empty());
    assert!(translate_key(WM_CHAR, 0x41, 1, &none, &mut dead).is_empty());
}

/// `TranslateMessage.yaml`: the SYS variants, every letter one lower-case
/// character, every digit one character.
#[test]
fn translate_key_sys_and_letters() {
    let none = [0u8; 256];
    let mut dead = DeadState::default();
    let m: Vec<CharMsg> = translate_key(WM_SYSKEYDOWN, 0x20, 0x2039_0001, &none, &mut dead).iter().collect();
    assert_eq!(m, [CharMsg { msg: WM_SYSCHAR, wparam: 0x20, lparam: 0x2039_0001 }]);
    let mut alt = [0u8; 256];
    alt[0x12] = 0x80;
    let m: Vec<CharMsg> = translate_key(WM_SYSKEYDOWN, 0xDC, 0x2029_0001, &alt, &mut dead).iter().collect();
    assert_eq!(m, [CharMsg { msg: WM_SYSDEADCHAR, wparam: 0x5E, lparam: 0x2029_0001 }]);
    dead.clear();
    for vk in 0x41..=0x5Au64 {
        let m: Vec<CharMsg> = translate_key(WM_KEYDOWN, vk, 1, &none, &mut dead).iter().collect();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].wparam, vk + 0x20);
    }
    for vk in 0x30..=0x39u64 {
        assert_eq!(translate_key(WM_KEYDOWN, vk, 1, &none, &mut dead).len(), 1);
    }
    for vk in [0x70u64, 0x25, 0x10] {
        assert!(translate_key(WM_KEYDOWN, vk, 1, &none, &mut dead).is_empty());
    }
}

#[test]
fn shift_grave_and_uppercase_combinations() {
    let mut dead = DeadState::default();
    let mut ks = [0u8; 256];
    let mut buf = [0u16; 4];
    assert_eq!(to_unicode(0xDC, 0x29, &ks, &mut buf, 0, &mut dead), -1);
    ks[0x10] = 0x80;
    assert_eq!(to_unicode(0x4F, 0x18, &ks, &mut buf, 0, &mut dead), 1);
    assert_eq!(buf[0], 0xD4);
    // Shift+^ is the degree sign, not a dead key.
    assert_eq!(to_unicode(0xDC, 0x29, &ks, &mut buf, 0, &mut dead), 1);
    assert_eq!(buf[0], 0xB0);
}

/// The US layout (ADR 0209): its keys' places are Learn's (the `VK_OEM_*` names for the US
/// keyboard, the scan code table), its characters the US legends.
#[test]
fn the_us_layout_reads_keys_by_their_us_places() {
    let us = Layout::Us;
    let vk = |scan| map_virtual_key_in(us, scan, MAPVK_VSC_TO_VK);
    // QWERTY: Y and Z in their places; the punctuation keys are the US ones.
    assert_eq!((vk(0x15), vk(0x2C)), (0x59, 0x5A));
    assert_eq!((vk(0x0C), vk(0x0D), vk(0x1A), vk(0x1B)), (VK_OEM_MINUS, VK_OEM_PLUS, VK_OEM_4, VK_OEM_6));
    assert_eq!((vk(0x27), vk(0x28), vk(0x29), vk(0x2B), vk(0x35)), (VK_OEM_1, VK_OEM_7, VK_OEM_3, VK_OEM_5, VK_OEM_2));
    // The other keys as on the German layout.
    assert_eq!((vk(0x1E), vk(0x02), vk(0x34), vk(0x56)), (0x41, 0x31, VK_OEM_PERIOD, VK_OEM_102));
    assert_eq!(map_virtual_key_in(us, VK_OEM_2, MAPVK_VK_TO_VSC), 0x35);
    assert_eq!(map_virtual_key_in(us, 0x5A, MAPVK_VK_TO_VSC), 0x2C);
    assert_eq!(map_virtual_key_in(us, VK_OEM_1, MAPVK_VK_TO_CHAR), ';' as u32);
    // The German layout is unchanged.
    assert_eq!(map_virtual_key_in(Layout::German, 0x15, MAPVK_VSC_TO_VK), 0x5A);
    assert_eq!(map_virtual_key_in(Layout::German, 0x35, MAPVK_VSC_TO_VK), VK_OEM_MINUS);

    // A URL types as it reads: `http://a-b.c/?x=1`.
    let mut shift = [0u8; 256];
    shift[VK_SHIFT as usize] = 0x80;
    let plain = [0u8; 256];
    let mut dead = DeadState::default();
    let mut c = |vk: u32, ks: &[u8; 256]| {
        let mut buf = [0u16; 2];
        assert_eq!(to_unicode_in(us, vk, map_virtual_key_in(us, vk, MAPVK_VK_TO_VSC), ks, &mut buf, 0, &mut dead), 1);
        char::from_u32(u32::from(buf[0])).unwrap()
    };
    let typed: String = [
        (0x48, &plain),
        (0x54, &plain),
        (0x54, &plain),
        (0x50, &plain),
        (VK_OEM_1, &shift),
        (VK_OEM_2, &plain),
        (VK_OEM_2, &plain),
        (0x41, &plain),
        (VK_OEM_MINUS, &plain),
        (0x42, &plain),
        (VK_OEM_PERIOD, &plain),
        (0x43, &plain),
        (VK_OEM_2, &plain),
        (VK_OEM_2, &shift),
        (0x58, &plain),
        (VK_OEM_PLUS, &plain),
        (0x31, &plain),
    ]
    .iter()
    .map(|&(vk, ks)| c(vk, ks))
    .collect();
    assert_eq!(typed, "http://a-b.c/?x=1");
    // No AltGr, no dead keys; Caps Lock only on letters.
    let mut altgr = [0u8; 256];
    altgr[VK_CONTROL as usize] = 0x80;
    altgr[VK_MENU as usize] = 0x80;
    let mut buf = [0u16; 2];
    assert_eq!(to_unicode_in(us, 0x51, 0x10, &altgr, &mut buf, 0, &mut dead), 0);
    let mut caps = [0u8; 256];
    caps[VK_CAPITAL as usize] = 1;
    assert_eq!(to_unicode_in(us, 0x31, 0x02, &caps, &mut buf, 0, &mut dead), 1);
    assert_eq!(buf[0], '1' as u16);
    assert_eq!(to_unicode_in(us, 0x41, 0x1E, &caps, &mut buf, 0, &mut dead), 1);
    assert_eq!(buf[0], 'A' as u16);
    // TranslateMessage's characters follow it too.
    let msgs = translate_key_in(us, WM_KEYDOWN, u64::from(VK_OEM_2), 0x0035_0001, &plain, &mut dead);
    assert_eq!(msgs.iter().next().map(|m| m.wparam), Some('/' as u64));
}

#[test]
fn the_desktops_layout_names_pick_a_layout() {
    assert_eq!(Layout::for_desktop("us"), Layout::Us);
    assert_eq!(Layout::for_desktop("US"), Layout::Us);
    assert_eq!(Layout::for_desktop("de"), Layout::German);
    assert_eq!(Layout::for_desktop("de-ch"), Layout::SwissGerman);
    assert_eq!(Layout::for_desktop("fr"), Layout::French);
    for l in [Layout::German, Layout::Us, Layout::SwissGerman, Layout::French] {
        assert_eq!(Layout::from_id(u64::from(l.id())), l);
    }
    assert_eq!((Layout::German.hkl(), Layout::Us.hkl()), (HKL, 0x0409_0409));
    assert_eq!((Layout::SwissGerman.hkl(), Layout::French.hkl()), (0x0807_0807, 0x040C_040C));
    assert_eq!(Layout::for_hkl(0x0807_0809), Layout::SwissGerman);
    assert_eq!(Layout::for_hkl(0x0407_2000), Layout::German);
    assert_eq!(Layout::for_hkl(0x0809_0809), Layout::Us);
    assert_eq!((Layout::German.klid(), Layout::Us.klid()), ("00000407", "00000409"));
}

/// A key state from a modifier mask: 1 Shift, 2 Ctrl, 4 Alt, 8 Caps Lock.
fn state(m: u32) -> [u8; 256] {
    let mut ks = [0u8; 256];
    if m & 1 != 0 {
        ks[0x10] = 0x80;
    }
    if m & 2 != 0 {
        ks[0x11] = 0x80;
    }
    if m & 4 != 0 {
        ks[0x12] = 0x80;
    }
    if m & 8 != 0 {
        ks[0x14] = 1;
    }
    ks
}

/// A cell as the tests print it: `-` none, `~xxxx` dead, `xxxx` one unit.
fn cell(l: Layout, vk: u32, m: u32) -> String {
    let mut buf = [0u16; 8];
    let mut dead = DeadState::default();
    let r = to_unicode_in(l, vk, 0, &state(m), &mut buf, TU_NO_STATE_CHANGE, &mut dead);
    match r {
        0 => "-".into(),
        r if r < 0 => format!("~{:04x}", buf[0]),
        _ => format!("{:04x}", buf[0]),
    }
}

/// The modifier masks of the ten columns of a table row: N S C SC Ct CtS AG AGS Al AGC.
const COLUMNS: [u32; 10] = [0, 1, 8, 9, 2, 3, 6, 7, 4, 14];

fn row_text(l: Layout, vk: u32) -> String {
    COLUMNS.iter().map(|&m| cell(l, vk, m)).collect::<Vec<_>>().join(" ")
}

#[test]
fn swiss_german_rows_per_the_sheet() {
    let l = Layout::SwissGerman;
    // From `tue_dech_table`'s recorded results.
    assert_eq!(row_text(l, 0x32), "0032 0022 0032 0022 - - 0040 - 0032 0040");
    assert_eq!(row_text(l, 0x36), "0036 0026 0036 0026 - 001e 00ac - 0036 00ac");
    assert_eq!(row_text(l, 0xBA), "00fc 00e8 00dc 00c8 001b - 005b - 00fc 005b");
    assert_eq!(row_text(l, 0xBB), "- - - - - - - - - -");
    assert_eq!(row_text(l, 0xC0), "~00a8 0021 ~00a8 0021 001d - 005d - ~00a8 005d");
    assert_eq!(row_text(l, 0xDB), "0027 003f 0027 003f - - ~00b4 - 0027 ~00b4");
    assert_eq!(row_text(l, 0xDC), "00e4 00e0 00c4 00c0 - - 007b - 00e4 007b");
    assert_eq!(row_text(l, 0xDD), "~005e ~0060 ~005e ~0060 - - ~007e - ~005e ~007e");
    assert_eq!(row_text(l, 0xDE), "00f6 00e9 00d6 00c9 - - - - 00f6 -");
    assert_eq!(row_text(l, 0xDF), "0024 00a3 0024 00a3 001c - 007d - 0024 007d");
    assert_eq!(row_text(l, 0xE2), "003c 003e 003c 003e 001c - 005c - 003c 005c");
    assert_eq!(row_text(l, 0x6E), "002e 002e 002e 002e - - - - 002e -");
    // Caps Lock does not shift the digits; it does the letters.
    assert_eq!(row_text(l, 0x31), "0031 002b 0031 002b - - 00a6 - 0031 00a6");
    assert_eq!(row_text(l, 0x5A), "007a 005a 005a 007a 001a 001a - - 007a -");
}

#[test]
fn swiss_german_types_the_address_bar_characters() {
    // z y ü ä é @ (AltGr+2): the places of the keyboard (scan codes) through the layout.
    let l = Layout::SwissGerman;
    let vk = |scan| map_virtual_key_in(l, scan, MAPVK_VSC_TO_VK);
    assert_eq!(vk(0x15), 0x5A); // the Z key's place is the one German calls Z
    assert_eq!(vk(0x2C), 0x59);
    assert_eq!(vk(0x1A), VK_OEM_1);
    assert_eq!(vk(0x28), VK_OEM_5);
    assert_eq!(vk(0x27), VK_OEM_7);
    let mut dead = DeadState::default();
    let mut buf = [0u16; 4];
    let mut t = |vk: u32, m: u32| {
        let r = to_unicode_in(l, vk, 0, &state(m), &mut buf, 0, &mut dead);
        assert_eq!(r, 1, "vk {vk:#x}");
        buf[0]
    };
    assert_eq!(t(vk(0x15), 0), 'z' as u16);
    assert_eq!(t(vk(0x2C), 0), 'y' as u16);
    assert_eq!(t(vk(0x1A), 0), 0xFC);
    assert_eq!(t(vk(0x28), 0), 0xE4);
    assert_eq!(t(vk(0x27), 1), 0xE9);
    assert_eq!(t(0x32, 6), '@' as u16);
}

#[test]
fn german_and_swiss_german_differ_as_the_sheet_says() {
    let (de, ch) = (Layout::German, Layout::SwissGerman);
    // Digits under Caps Lock.
    assert_eq!(cell(de, 0x30, 8), "003d");
    assert_eq!(cell(ch, 0x30, 8), "0030");
    // The decimal key.
    assert_eq!(cell(de, VK_DECIMAL, 0), "002c");
    assert_eq!(cell(ch, VK_DECIMAL, 0), "002e");
    // VK_OEM_PLUS has no key in Swiss German.
    assert_eq!(map_virtual_key_in(ch, VK_OEM_PLUS, MAPVK_VK_TO_VSC), 0);
    assert_eq!(map_virtual_key_in(de, VK_OEM_PLUS, MAPVK_VK_TO_VSC), 0x1B);
    // AltGr+Q is @ in German, nothing in Swiss German.
    assert_eq!(cell(de, 0x51, 6), "0040");
    assert_eq!(cell(ch, 0x51, 6), "-");
}

#[test]
fn dead_keys_of_the_new_layouts_combine() {
    let l = Layout::SwissGerman;
    let mut dead = DeadState::default();
    let press = |vk: u32, m: u32, dead: &mut DeadState| {
        let mut buf = [0u16; 4];
        let r = to_unicode_in(l, vk, 0, &state(m), &mut buf, 0, dead);
        (r, buf)
    };
    // Dead diaeresis, then u: ü.
    assert_eq!(press(0xC0, 0, &mut dead).0, -1);
    assert_eq!(press(0x55, 0, &mut dead), (1, [0xFC, 0, 0, 0]));
    // Dead diaeresis, then n: the accent and n.
    press(0xC0, 0, &mut dead);
    assert_eq!(press(0x4E, 0, &mut dead), (2, [0xA8, 'n' as u16, 0, 0]));
    // Dead tilde (AltGr+^), then n: ñ.
    assert_eq!(press(0xDD, 6, &mut dead).0, -1);
    assert_eq!(press(0x4E, 0, &mut dead), (1, [0xF1, 0, 0, 0]));
    // A dead key, then Ctrl+a: U+0001 alone; then AltGr+a: nothing, and the accent is gone.
    press(0xC0, 0, &mut dead);
    assert_eq!(press(0x41, 2, &mut dead), (1, [1, 0, 0, 0]));
    press(0xC0, 0, &mut dead);
    assert_eq!(press(0x41, 6, &mut dead).0, 0);
    assert_eq!(dead.pending(), None);
    // The pending dead key stays through a modifier.
    press(0xC0, 0, &mut dead);
    assert_eq!(press(VK_SHIFT, 1, &mut dead).0, 0);
    assert_eq!(dead.pending(), Some(0xA8));
}

#[test]
fn vk_key_scan_per_the_sheet() {
    let ex = |l, c: char| vk_key_scan_in(l, c as u16) as u16;
    let (us, de, ch) = (Layout::Us, Layout::German, Layout::SwissGerman);
    assert_eq!(ex(us, 'a'), 0x0041);
    assert_eq!(ex(us, 'A'), 0x0141);
    assert_eq!(ex(us, '"'), 0x01DE);
    assert_eq!(ex(us, '\\'), 0x00DC);
    assert_eq!(ex(us, '\u{1c}'), 0x02DC);
    assert_eq!(ex(us, '\0'), 0x0332);
    assert_eq!(ex(us, '\u{1e}'), 0x0336);
    assert_eq!(ex(us, '\u{1f}'), 0x03BD);
    assert_eq!(ex(us, '*'), 0x0138);
    assert_eq!(ex(de, '@'), 0x0651);
    assert_eq!(ex(de, '\u{20ac}'), 0x0645);
    assert_eq!(ex(de, '^'), 0x00DC);
    assert_eq!(ex(de, '/'), 0x0137);
    assert_eq!(ex(de, '\u{3}'), 0x0003);
    assert_eq!(ex(ch, '@'), 0x0632);
    assert_eq!(ex(ch, '{c4}'), 0x00DC);
    assert_eq!(ex(ch, '\u{e9}'), 0x01DE);
    assert_eq!(vk_key_scan_in(us, 0x20AC), -1);
    assert_eq!(vk_key_scan_in(us, 0xFC), -1);
}

#[test]
fn us_control_characters_per_the_sheet() {
    let us = Layout::Us;
    assert_eq!(cell(us, VK_OEM_4, 2), "001b");
    assert_eq!(cell(us, VK_OEM_6, 2), "001d");
    assert_eq!(cell(us, VK_OEM_5, 2), "001c");
    assert_eq!(cell(us, VK_OEM_102, 2), "001c");
    assert_eq!(cell(us, 0x32, 3), "0000");
    assert_eq!(cell(us, 0x36, 3), "001e");
    assert_eq!(cell(us, VK_OEM_MINUS, 3), "001f");
    assert_eq!(cell(us, VK_OEM_4, 3), "-");
    assert_eq!(cell(us, 0x41, 6), "-");
}
