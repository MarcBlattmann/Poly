//! Encoding tests for the emitter against the system assembler.
//!
//! [`cases`] pairs each emitter call with the same instruction in Intel
//! syntax. `vectors.txt` holds what the system assembler (`clang`, a black
//! box) makes of each text; the test `x64_vectors` checks the emitter
//! against it on every host. After adding cases, regenerate it on a host
//! whose `clang` targets x86-64 (any Mac, most Linux hosts):
//!
//! ```text
//! cargo test -p poly-a64-jit -- --ignored regenerate_x64_vectors
//! ```

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::collections::BTreeMap;

use super::*;

type Case = (String, Box<dyn Fn(&mut Asm)>);

const REGS: [R; 16] = [
    R::Rax,
    R::Rcx,
    R::Rdx,
    R::Rbx,
    R::Rsp,
    R::Rbp,
    R::Rsi,
    R::Rdi,
    R::R8,
    R::R9,
    R::R10,
    R::R11,
    R::R12,
    R::R13,
    R::R14,
    R::R15,
];

fn name(r: R, sz: Sz) -> String {
    const Q: [&str; 16] =
        ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];
    const D: [&str; 16] = [
        "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d", "r12d", "r13d", "r14d",
        "r15d",
    ];
    const H: [&str; 16] =
        ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di", "r8w", "r9w", "r10w", "r11w", "r12w", "r13w", "r14w", "r15w"];
    const B: [&str; 16] = [
        "al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b", "r12b", "r13b", "r14b",
        "r15b",
    ];
    let i = r as usize;
    String::from(match sz {
        Sz::Q => Q[i],
        Sz::D => D[i],
        Sz::H => H[i],
        Sz::B => B[i],
    })
}

fn ptr(sz: Sz) -> &'static str {
    match sz {
        Sz::Q => "qword ptr",
        Sz::D => "dword ptr",
        Sz::H => "word ptr",
        Sz::B => "byte ptr",
    }
}

fn mtext(m: M) -> String {
    let mut s = format!("[{}", name(m.base, Sz::Q));
    if let Some((i, sc)) = m.index {
        s += &format!(" + {}*{}", name(i, Sz::Q), 1 << sc);
    }
    if m.disp != 0 {
        s += &format!(" + {}", m.disp);
    }
    s + "]"
}

const SIZES: [Sz; 4] = [Sz::B, Sz::H, Sz::D, Sz::Q];
const DISPS: [i32; 4] = [0, 8, -128, 0x1234];

fn mems() -> Vec<M> {
    let mut v = Vec::new();
    for &b in &REGS {
        for &d in &DISPS {
            v.push(mem(b, d));
        }
    }
    for &b in &[R::Rax, R::Rsp, R::Rbp, R::R12, R::R13, R::R15] {
        for &i in &[R::Rcx, R::R9, R::Rbp, R::R13] {
            v.push(M { base: b, index: Some((i, 0)), disp: 0 });
            v.push(M { base: b, index: Some((i, 3)), disp: 16 });
        }
    }
    v
}

/// Every case: (Intel syntax, emitter call).
pub(crate) fn cases() -> Vec<Case> {
    let mut v: Vec<Case> = Vec::new();
    let alus = [
        (Alu::Add, "add"),
        (Alu::Or, "or"),
        (Alu::Adc, "adc"),
        (Alu::Sbb, "sbb"),
        (Alu::And, "and"),
        (Alu::Sub, "sub"),
        (Alu::Xor, "xor"),
        (Alu::Cmp, "cmp"),
    ];
    for &sz in &SIZES {
        for &a in &REGS {
            for &b in &REGS {
                v.push((
                    format!("mov {}, {}", name(a, sz), name(b, sz)),
                    Box::new(move |s: &mut Asm| s.mov_rr(sz, a, b)),
                ));
            }
            for &(op, n) in &alus {
                let b = REGS[(a as usize * 5 + op as usize) % 16];
                v.push((
                    format!("{n} {}, {}", name(a, sz), name(b, sz)),
                    Box::new(move |s: &mut Asm| s.alu_rr(sz, op, a, b)),
                ));
                for imm in [1i32, -1, 127, -128, 0x1234_5678] {
                    let imm = if sz == Sz::B {
                        imm as i8 as i32
                    } else if sz == Sz::H {
                        imm as i16 as i32
                    } else {
                        imm
                    };
                    v.push((
                        format!("{n} {}, {imm}", name(a, sz)),
                        Box::new(move |s: &mut Asm| s.alu_ri(sz, op, a, imm)),
                    ));
                }
            }
            v.push((
                format!("test {}, {}", name(a, sz), name(R::R9, sz)),
                Box::new(move |s: &mut Asm| s.test_rr(sz, a, R::R9)),
            ));
            v.push((format!("not {}", name(a, sz)), Box::new(move |s: &mut Asm| s.not(sz, a))));
            v.push((format!("neg {}", name(a, sz)), Box::new(move |s: &mut Asm| s.neg(sz, a))));
            if sz != Sz::B {
                v.push((format!("mul {}", name(a, sz)), Box::new(move |s: &mut Asm| s.mul(sz, a))));
                v.push((format!("imul {}", name(a, sz)), Box::new(move |s: &mut Asm| s.imul1(sz, a))));
                v.push((format!("div {}", name(a, sz)), Box::new(move |s: &mut Asm| s.div(sz, a))));
                v.push((format!("idiv {}", name(a, sz)), Box::new(move |s: &mut Asm| s.idiv(sz, a))));
                let b = REGS[(a as usize + 3) % 16];
                v.push((
                    format!("imul {}, {}", name(a, sz), name(b, sz)),
                    Box::new(move |s: &mut Asm| s.imul(sz, a, b)),
                ));
                v.push((format!("bsr {}, {}", name(a, sz), name(b, sz)), Box::new(move |s: &mut Asm| s.bsr(sz, a, b))));
                for (cc, n) in [(Cc::E, "e"), (Cc::B, "b"), (Cc::G, "g"), (Cc::Np, "np")] {
                    v.push((
                        format!("cmov{n} {}, {}", name(a, sz), name(b, sz)),
                        Box::new(move |s: &mut Asm| s.cmov(sz, cc, a, b)),
                    ));
                }
                v.push((format!("bt {}, 5", name(a, sz)), Box::new(move |s: &mut Asm| s.bt_ri(sz, a, 5))));
            }
            if sz == Sz::D || sz == Sz::Q {
                v.push((format!("bswap {}", name(a, sz)), Box::new(move |s: &mut Asm| s.bswap(sz, a))));
            }
            for (op, n) in [(Sh::Rol, "rol"), (Sh::Ror, "ror"), (Sh::Shl, "shl"), (Sh::Shr, "shr"), (Sh::Sar, "sar")] {
                v.push((format!("{n} {}, cl", name(a, sz)), Box::new(move |s: &mut Asm| s.shift_cl(sz, op, a))));
                for k in [1u8, 7] {
                    v.push((
                        format!("{n} {}, {k}", name(a, sz)),
                        Box::new(move |s: &mut Asm| s.shift_ri(sz, op, a, k)),
                    ));
                }
            }
        }
        for m in mems() {
            let r = REGS[(m.disp as usize ^ m.base as usize) % 16];
            let ms = mtext(m);
            v.push((format!("mov {}, {} {ms}", name(r, sz), ptr(sz)), Box::new(move |s: &mut Asm| s.load(sz, r, m))));
            v.push((format!("mov {} {ms}, {}", ptr(sz), name(r, sz)), Box::new(move |s: &mut Asm| s.store(sz, m, r))));
            let imm = match sz {
                Sz::B => 0x5A,
                Sz::H => 0x1234,
                _ => -5,
            };
            v.push((format!("mov {} {ms}, {imm}", ptr(sz)), Box::new(move |s: &mut Asm| s.store_imm(sz, m, imm))));
            v.push((
                format!("add {}, {} {ms}", name(r, sz), ptr(sz)),
                Box::new(move |s: &mut Asm| s.alu_rm(sz, Alu::Add, r, m)),
            ));
            v.push((
                format!("sub {} {ms}, {}", ptr(sz), name(r, sz)),
                Box::new(move |s: &mut Asm| s.alu_mr(sz, Alu::Sub, m, r)),
            ));
            v.push((format!("cmp {} {ms}, 3", ptr(sz)), Box::new(move |s: &mut Asm| s.alu_mi(sz, Alu::Cmp, m, 3))));
            v.push((
                format!("xchg {} {ms}, {}", ptr(sz), name(r, sz)),
                Box::new(move |s: &mut Asm| s.xchg_mr(sz, m, r)),
            ));
            v.push((
                format!("lock cmpxchg {} {ms}, {}", ptr(sz), name(r, sz)),
                Box::new(move |s: &mut Asm| s.lock_cmpxchg(sz, m, r)),
            ));
            v.push((
                format!("lock xadd {} {ms}, {}", ptr(sz), name(r, sz)),
                Box::new(move |s: &mut Asm| s.lock_xadd(sz, m, r)),
            ));
            if sz == Sz::D || sz == Sz::Q {
                v.push((format!("lea {}, {ms}", name(r, sz)), Box::new(move |s: &mut Asm| s.lea(sz, r, m))));
            }
            if sz == Sz::B || sz == Sz::H {
                v.push((
                    format!("movzx {}, {} {ms}", name(r, Sz::D), ptr(sz)),
                    Box::new(move |s: &mut Asm| s.load_zx(sz, r, m)),
                ));
            }
            if sz != Sz::Q {
                for to in [Sz::D, Sz::Q] {
                    if sz == Sz::D && to == Sz::D {
                        continue;
                    }
                    let mn = if sz == Sz::D { "movsxd" } else { "movsx" };
                    v.push((
                        format!("{mn} {}, {} {ms}", name(r, to), ptr(sz)),
                        Box::new(move |s: &mut Asm| s.load_sx(sz, to, r, m)),
                    ));
                }
            }
        }
    }
    for &a in &REGS {
        for &b in &[R::Rax, R::Rsp, R::Rsi, R::R8, R::R15] {
            v.push((
                format!("movzx {}, {}", name(a, Sz::D), name(b, Sz::B)),
                Box::new(move |s: &mut Asm| s.zx_rr(Sz::B, a, b)),
            ));
            v.push((
                format!("movzx {}, {}", name(a, Sz::D), name(b, Sz::H)),
                Box::new(move |s: &mut Asm| s.zx_rr(Sz::H, a, b)),
            ));
            for to in [Sz::D, Sz::Q] {
                v.push((
                    format!("movsx {}, {}", name(a, to), name(b, Sz::B)),
                    Box::new(move |s: &mut Asm| s.sx_rr(Sz::B, to, a, b)),
                ));
                v.push((
                    format!("movsx {}, {}", name(a, to), name(b, Sz::H)),
                    Box::new(move |s: &mut Asm| s.sx_rr(Sz::H, to, a, b)),
                ));
            }
            v.push((
                format!("movsxd {}, {}", name(a, Sz::Q), name(b, Sz::D)),
                Box::new(move |s: &mut Asm| s.sx_rr(Sz::D, Sz::Q, a, b)),
            ));
        }
        for imm in
            [0u64, 1, 0xFFFF_FFFF, 0xFFFF_FFFF_FFFF_FFF0, 0x1_0000_0000, 0x8000_0000_0000_0000, 0x1234_5678_9ABC_DEF0]
        {
            let text = if imm == 0 {
                format!("xor {}, {}", name(a, Sz::D), name(a, Sz::D))
            } else if imm <= 0xFFFF_FFFF {
                format!("mov {}, {imm}", name(a, Sz::D))
            } else if (imm as i64) < 0 && (imm as i64) >= i64::from(i32::MIN) {
                format!("mov {}, {}", name(a, Sz::Q), imm as i64)
            } else {
                format!("movabs {}, {imm}", name(a, Sz::Q))
            };
            v.push((text, Box::new(move |s: &mut Asm| s.mov_imm(a, imm))));
        }
        for (cc, n) in [(Cc::O, "o"), (Cc::B, "b"), (Cc::E, "e"), (Cc::S, "s"), (Cc::L, "l"), (Cc::Ae, "ae")] {
            v.push((format!("set{n} {}", name(a, Sz::B)), Box::new(move |s: &mut Asm| s.setcc(cc, a))));
        }
        v.push((format!("push {}", name(a, Sz::Q)), Box::new(move |s: &mut Asm| s.push(a))));
        v.push((format!("pop {}", name(a, Sz::Q)), Box::new(move |s: &mut Asm| s.pop(a))));
        v.push((format!("jmp {}", name(a, Sz::Q)), Box::new(move |s: &mut Asm| s.jmp_r(a))));
        v.push((format!("call {}", name(a, Sz::Q)), Box::new(move |s: &mut Asm| s.call_r(a))));
        v.push((
            format!("test {}, 1048576", name(a, Sz::D)),
            Box::new(move |s: &mut Asm| s.test_ri(Sz::D, a, 1 << 20)),
        ));
        v.push((format!("test {}, 3", name(a, Sz::B)), Box::new(move |s: &mut Asm| s.test_ri(Sz::B, a, 3))));
    }
    for m in mems() {
        let ms = mtext(m);
        v.push((format!("jmp qword ptr {ms}"), Box::new(move |s: &mut Asm| s.jmp_m(m))));
        v.push((format!("test byte ptr {ms}, 24"), Box::new(move |s: &mut Asm| s.test_mi8(m, 24))));
        for x in [0u8, 7, 8, 15] {
            let xs = format!("xmm{x}");
            v.push((format!("movdqu {xs}, xmmword ptr {ms}"), Box::new(move |s: &mut Asm| s.movdqu_load(Xmm(x), m))));
            v.push((format!("movdqu xmmword ptr {ms}, {xs}"), Box::new(move |s: &mut Asm| s.movdqu_store(m, Xmm(x)))));
            v.push((format!("movsd {xs}, qword ptr {ms}"), Box::new(move |s: &mut Asm| s.movsd_load(Xmm(x), m))));
            v.push((format!("movsd qword ptr {ms}, {xs}"), Box::new(move |s: &mut Asm| s.movsd_store(m, Xmm(x)))));
            v.push((format!("movss {xs}, dword ptr {ms}"), Box::new(move |s: &mut Asm| s.movss_load(Xmm(x), m))));
            v.push((format!("movss dword ptr {ms}, {xs}"), Box::new(move |s: &mut Asm| s.movss_store(m, Xmm(x)))));
        }
    }
    for &r in &REGS {
        for x in [0u8, 3, 9, 15] {
            v.push((format!("movq xmm{x}, {}", name(r, Sz::Q)), Box::new(move |s: &mut Asm| s.movq_xr(Xmm(x), r))));
            v.push((format!("movq {}, xmm{x}", name(r, Sz::Q)), Box::new(move |s: &mut Asm| s.movq_rx(r, Xmm(x)))));
        }
    }
    for (d, sx) in [(0u8, 1u8), (9, 2), (3, 12), (15, 15)] {
        for (op, n) in [(0x58u8, "add"), (0x59, "mul"), (0x5C, "sub"), (0x5E, "div"), (0x51, "sqrt")] {
            v.push((
                format!("{n}sd xmm{d}, xmm{sx}"),
                Box::new(move |s: &mut Asm| s.sse_scalar(true, op, Xmm(d), Xmm(sx))),
            ));
            v.push((
                format!("{n}ss xmm{d}, xmm{sx}"),
                Box::new(move |s: &mut Asm| s.sse_scalar(false, op, Xmm(d), Xmm(sx))),
            ));
        }
        v.push((format!("ucomisd xmm{d}, xmm{sx}"), Box::new(move |s: &mut Asm| s.ucomis(true, Xmm(d), Xmm(sx)))));
        v.push((format!("ucomiss xmm{d}, xmm{sx}"), Box::new(move |s: &mut Asm| s.ucomis(false, Xmm(d), Xmm(sx)))));
        for (op, n) in [
            (0xFEu8, "paddd"),
            (0xD4, "paddq"),
            (0xFC, "paddb"),
            (0xFB, "psubq"),
            (0xDB, "pand"),
            (0xEB, "por"),
            (0xEF, "pxor"),
            (0x74, "pcmpeqb"),
        ] {
            v.push((format!("{n} xmm{d}, xmm{sx}"), Box::new(move |s: &mut Asm| s.sse_packed(op, Xmm(d), Xmm(sx)))));
        }
    }
    for m in mems() {
        let ms = mtext(m);
        v.push((format!("bt dword ptr {ms}, 29"), Box::new(move |s: &mut Asm| s.bt_mi(Sz::D, m, 29))));
    }
    for &r in &REGS {
        v.push((format!("mov {}, 0", name(r, Sz::D)), Box::new(move |s: &mut Asm| s.mov_r32_imm(r, 0))));
    }
    for &r in &REGS {
        for x in [0u8, 9] {
            for (d, n) in [(true, "sd"), (false, "ss")] {
                for (w64, rn) in [(false, name(r, Sz::D)), (true, name(r, Sz::Q))] {
                    v.push((
                        format!("cvtsi2{n} xmm{x}, {rn}"),
                        Box::new(move |s: &mut Asm| s.cvtsi2(d, w64, Xmm(x), r)),
                    ));
                    v.push((
                        format!("cvtt{n}2si {rn}, xmm{x}"),
                        Box::new(move |s: &mut Asm| s.cvtt2si(d, w64, r, Xmm(x))),
                    ));
                }
            }
            v.push((format!("movd xmm{x}, {}", name(r, Sz::D)), Box::new(move |s: &mut Asm| s.movd_xr(Xmm(x), r))));
            v.push((format!("movd {}, xmm{x}", name(r, Sz::D)), Box::new(move |s: &mut Asm| s.movd_rx(r, Xmm(x)))));
        }
        for (sz, n) in [(Sz::B, "byte"), (Sz::H, "word"), (Sz::D, "dword"), (Sz::Q, "qword")] {
            let _ = n;
            let dst = if sz == Sz::Q { name(R::R9, Sz::Q) } else { name(R::R9, Sz::D) };
            v.push((format!("crc32 {dst}, {}", name(r, sz)), Box::new(move |s: &mut Asm| s.crc32(sz, R::R9, r))));
        }
    }
    for (d, a, b) in [(0u8, 1u8, 2u8), (9, 3, 12), (15, 15, 15), (2, 8, 1)] {
        v.push((format!("cvtsd2ss xmm{d}, xmm{b}"), Box::new(move |s: &mut Asm| s.cvt_fp(true, Xmm(d), Xmm(b)))));
        v.push((format!("cvtss2sd xmm{d}, xmm{b}"), Box::new(move |s: &mut Asm| s.cvt_fp(false, Xmm(d), Xmm(b)))));
        for (op, n) in [(0xB9u8, "vfmadd231"), (0xBB, "vfmsub231"), (0xBD, "vfnmadd231"), (0xBF, "vfnmsub231")] {
            v.push((
                format!("{n}sd xmm{d}, xmm{a}, xmm{b}"),
                Box::new(move |s: &mut Asm| s.vfma231(true, op, Xmm(d), Xmm(a), Xmm(b))),
            ));
            v.push((
                format!("{n}ss xmm{d}, xmm{a}, xmm{b}"),
                Box::new(move |s: &mut Asm| s.vfma231(false, op, Xmm(d), Xmm(a), Xmm(b))),
            ));
        }
        for (op, n) in [
            (0xF8u8, "psubb"),
            (0xF9, "psubw"),
            (0xFA, "psubd"),
            (0xFD, "paddw"),
            (0xDF, "pandn"),
            (0x75, "pcmpeqw"),
            (0x76, "pcmpeqd"),
        ] {
            v.push((format!("{n} xmm{d}, xmm{b}"), Box::new(move |s: &mut Asm| s.sse_packed(op, Xmm(d), Xmm(b)))));
        }
    }
    v.push(("cmc".into(), Box::new(|s: &mut Asm| s.cmc())));
    v.push(("pushfq".into(), Box::new(|s: &mut Asm| s.pushfq())));
    v.push(("ret".into(), Box::new(|s: &mut Asm| s.ret())));
    v.push(("cqo".into(), Box::new(|s: &mut Asm| s.cqo(Sz::Q))));
    v.push(("cdq".into(), Box::new(|s: &mut Asm| s.cdq_())));
    v.push(("ud2".into(), Box::new(|s: &mut Asm| s.ud2())));
    v
}

impl Asm {
    fn cdq_(&mut self) {
        self.cqo(Sz::D);
    }
}

fn load_vectors() -> BTreeMap<String, Vec<u8>> {
    let text = include_str!("vectors.txt");
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let Some((hex, asm)) = line.split_once('\t') else { continue };
        let bytes = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        m.insert(String::from(asm), bytes);
    }
    m
}

#[test]
fn x64_vectors() {
    let vectors = load_vectors();
    let mut missing = 0;
    let mut bad = Vec::new();
    let all = cases();
    for (text, f) in &all {
        let mut a = Asm::new();
        f(&mut a);
        match vectors.get(text) {
            None => missing += 1,
            Some(want) if *want != a.buf => bad.push(format!("{text}: emitter {:02x?} assembler {:02x?}", a.buf, want)),
            Some(_) => {}
        }
    }
    assert!(bad.is_empty(), "{} of {} encodings differ:\n{}", bad.len(), all.len(), bad.join("\n"));
    assert_eq!(missing, 0, "{missing} cases have no vector: regenerate vectors.txt (see the module docs)");
}

/// Assemble every case with the system `clang` and rewrite `vectors.txt`.
#[test]
#[ignore = "needs clang with an x86-64 target; rewrites vectors.txt"]
fn regenerate_x64_vectors() {
    use std::process::Command;
    let all = cases();
    let dir = std::env::temp_dir().join(format!("x64vec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut src = String::from(".intel_syntax noprefix\n.text\n");
    for (i, (text, _)) in all.iter().enumerate() {
        src += &format!("_v{i}:\n    {text}\n");
    }
    src += &format!("_v{}:\n", all.len());
    std::fs::write(dir.join("v.s"), &src).unwrap();
    let triple = if cfg!(target_os = "macos") { "x86_64-apple-macos" } else { "x86_64-linux-gnu" };
    let ok = Command::new("clang")
        .args(["-c", "-target", triple, "-x", "assembler"])
        .arg(dir.join("v.s"))
        .arg("-o")
        .arg(dir.join("v.o"))
        .status()
        .unwrap()
        .success();
    assert!(ok, "clang failed");
    let obj = std::fs::read(dir.join("v.o")).unwrap();
    let (text, syms) = object_text_and_symbols(&obj);
    let mut out = String::new();
    for (i, (t, _)) in all.iter().enumerate() {
        let a = syms[&format!("_v{i}")] as usize;
        let b = syms[&format!("_v{}", i + 1)] as usize;
        let hex: String = text[a..b].iter().map(|x| format!("{x:02x}")).collect();
        out += &format!("{hex}\t{t}\n");
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/x64/vectors.txt");
    std::fs::write(path, out).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The text section and symbol values of a Mach-O or ELF object.
fn object_text_and_symbols(obj: &[u8]) -> (Vec<u8>, BTreeMap<String, u64>) {
    let u16_at = |o: usize| u16::from_le_bytes(obj[o..o + 2].try_into().unwrap());
    let u32_at = |o: usize| u32::from_le_bytes(obj[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(obj[o..o + 8].try_into().unwrap());
    let cstr = |o: usize| {
        let e = obj[o..].iter().position(|&b| b == 0).unwrap();
        String::from_utf8_lossy(&obj[o..o + e]).into_owned()
    };
    let mut syms = BTreeMap::new();
    if u32_at(0) == 0xFEED_FACF {
        let mut off = 32;
        let mut text = Vec::new();
        for _ in 0..u32_at(16) {
            let (cmd, size) = (u32_at(off), u32_at(off + 4) as usize);
            if cmd == 0x19 {
                for k in 0..u32_at(off + 64) as usize {
                    let so = off + 72 + 80 * k;
                    if obj[so..].starts_with(b"__text\0") {
                        let (sz, fo) = (u64_at(so + 40) as usize, u32_at(so + 48) as usize);
                        text = obj[fo..fo + sz].to_vec();
                    }
                }
            } else if cmd == 2 {
                let (so, n, st) = (u32_at(off + 8) as usize, u32_at(off + 12) as usize, u32_at(off + 16) as usize);
                for k in 0..n {
                    let e = so + 16 * k;
                    syms.insert(cstr(st + u32_at(e) as usize), u64_at(e + 8));
                }
            }
            off += size;
        }
        (text, syms)
    } else {
        // ELF64: section headers, .text and .symtab.
        let (shoff, shentsize, shnum, shstrndx) =
            (u64_at(0x28) as usize, u16_at(0x3A) as usize, u16_at(0x3C) as usize, u16_at(0x3E) as usize);
        let sh = |i: usize| shoff + i * shentsize;
        let shstr = u64_at(sh(shstrndx) + 0x18) as usize;
        let mut text = Vec::new();
        let mut text_index = 0;
        for i in 0..shnum {
            if cstr(shstr + u32_at(sh(i)) as usize) == ".text" {
                let (o, s) = (u64_at(sh(i) + 0x18) as usize, u64_at(sh(i) + 0x20) as usize);
                text = obj[o..o + s].to_vec();
                text_index = i;
            }
        }
        for i in 0..shnum {
            if u32_at(sh(i) + 4) == 2 {
                let (o, s, link) =
                    (u64_at(sh(i) + 0x18) as usize, u64_at(sh(i) + 0x20) as usize, u32_at(sh(i) + 0x28) as usize);
                let strtab = u64_at(sh(link) + 0x18) as usize;
                for e in (o..o + s).step_by(24) {
                    if u16_at(e + 6) as usize == text_index {
                        let n = cstr(strtab + u32_at(e) as usize);
                        syms.insert(n, u64_at(e + 8));
                    }
                }
            }
        }
        (text, syms)
    }
}
