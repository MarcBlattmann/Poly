//! `a64diff native`: the same generated one-instruction blocks as `fuzz`, run
//! on the host's own arm64 CPU instead of under qemu (macOS on Apple silicon).
//!
//! The host CPU is the oracle. Each case is assembled by the system `clang`
//! (a black box) into a Mach-O object; the tool copies the object's code
//! into a `MAP_JIT` mapping and runs each case in its own process: a
//! prologue loads the case's whole register state (`X0`-`X30`, `SP`,
//! `NZCV`, `FPCR`, `V0`-`V31`), the case runs, and a `BRK` ends it. A signal
//! handler catches the `BRK` (and any fault or undefined instruction), copies
//! the registers out of the signal context and resumes the tool. poly-a64
//! runs the same bytes at the same addresses against a copy of the memory,
//! and the two results are compared record by record, exactly as `fuzz`
//! compares qemu with poly-a64.
//!
//! Only the platform's differences are removed: `X18` (the platform register
//! the OS may change at any time) is not compared, `TPIDRRO_EL0` and
//! `DCZID_EL0` are read from the host and handed to poly-a64, and only the
//! `IOC` and `DZC` bits of `FPSR` are compared (poly-a64 tracks no others).
//! Pointer authentication is on in the host (macOS keys); for arm64 (not
//! arm64e) processes the `PAC*`/`AUT*` instructions leave registers alone,
//! which is what poly-a64 does.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::fuzz::{self, Case, FAMILIES, REC_BYTES, REC_WIN, REC_WORDS, Rng, State};

/// Bytes of the data region; `x5` points at its middle for memory cases.
const DATA: usize = 64 * 1024;
/// Offset of the bytes the state's `window` fills (`x5 - 256`).
const WIN_OFF: usize = DATA / 2 - 256;
/// The `BRK` immediate that ends a case.
const END_BRK: u32 = 0x7A7A;

/// One case's result: a record in `fuzz`'s layout, how it ended and where.
struct Run {
    rec: Vec<u64>,
    end: End,
    /// The data region after the case.
    data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// Reached the final `BRK`.
    Done,
    /// A data or instruction fault at this `PC`.
    Fault(u64),
    /// An undefined instruction at this `PC`.
    Undefined(u64),
    /// poly-a64 only: an instruction it does not implement.
    Unsupported(u64, u32),
    /// poly-a64 only: ran past the step limit or stopped elsewhere.
    Other(u64),
}

pub struct Opts {
    pub seed: u64,
    pub blocks: usize,
    pub family: Option<String>,
    pub keep: Option<std::path::PathBuf>,
    pub verbose: usize,
}

/// The prologue macro and the cases, in the syntax of Apple's assembler.
fn assembly(cases: &[(Case, State)]) -> String {
    let mut s = String::new();
    s.push_str(".text\n.macro LOADSTATE\n");
    s.push_str("    add x2, x0, #288\n");
    for k in 0..16 {
        s.push_str(&format!("    ldp q{}, q{}, [x2, #{}]\n", 2 * k, 2 * k + 1, 32 * k));
    }
    s.push_str("    ldr x2, [x0, #248]\n    mov sp, x2\n");
    s.push_str("    ldr x2, [x0, #256]\n    msr nzcv, x2\n");
    s.push_str("    ldr x2, [x0, #264]\n    msr fpcr, x2\n    msr fpsr, xzr\n");
    for k in 0..15 {
        let (a, b) = (1 + 2 * k, 2 + 2 * k);
        s.push_str(&format!("    ldp x{a}, x{b}, [x0, #{}]\n", 8 * a));
    }
    s.push_str("    ldr x0, [x0]\n.endm\n");
    for (i, (c, _)) in cases.iter().enumerate() {
        s.push_str(&format!(".p2align 4\n_c{i}:\n    LOADSTATE\n_s{i}:\n"));
        for l in c.setup.iter().chain(&c.body) {
            s.push_str("    ");
            s.push_str(l);
            s.push('\n');
        }
        s.push_str(&format!("_e{i}:\n    brk #{END_BRK:#x}\n"));
    }
    s
}

/// The `__text` bytes of a Mach-O object and the values of its symbols.
fn read_object(obj: &[u8]) -> Result<(Vec<u8>, BTreeMap<String, u64>), String> {
    let u32_at = |o: usize| u32::from_le_bytes(obj[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(obj[o..o + 8].try_into().unwrap());
    if obj.len() < 32 || u32_at(0) != 0xFEED_FACF {
        return Err("not a 64-bit Mach-O object".into());
    }
    let ncmds = u32_at(16) as usize;
    let mut off = 32;
    let mut text = None;
    let mut syms = BTreeMap::new();
    for _ in 0..ncmds {
        let (cmd, size) = (u32_at(off), u32_at(off + 4) as usize);
        if cmd == 0x19 {
            // LC_SEGMENT_64: sections follow the 72-byte command header.
            let nsects = u32_at(off + 64) as usize;
            for k in 0..nsects {
                let so = off + 72 + 80 * k;
                let name = &obj[so..so + 16];
                if name.starts_with(b"__text\0") {
                    let (addr, sz, fo, nreloc) =
                        (u64_at(so + 32), u64_at(so + 40) as usize, u32_at(so + 48) as usize, u32_at(so + 60));
                    if addr != 0 {
                        return Err("__text does not start at 0".into());
                    }
                    if nreloc != 0 {
                        return Err(format!("__text has {nreloc} relocations; cases must be position independent"));
                    }
                    text = Some(obj[fo..fo + sz].to_vec());
                }
            }
        } else if cmd == 0x2 {
            // LC_SYMTAB.
            let (symoff, nsyms, stroff) = (u32_at(off + 8) as usize, u32_at(off + 12) as usize, u32_at(off + 16) as usize);
            for k in 0..nsyms {
                let e = symoff + 16 * k;
                let strx = u32_at(e) as usize;
                let name_bytes = &obj[stroff + strx..];
                let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(0);
                let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
                syms.insert(name, u64_at(e + 8));
            }
        }
        off += size;
    }
    Ok((text.ok_or("no __text section")?, syms))
}

/// Memory for poly-a64: the code and the data region at the host's addresses.
struct TwoRegions<'a> {
    code_base: u64,
    code: &'a [u8],
    data_base: u64,
    data: Vec<u8>,
}

impl TwoRegions<'_> {
    fn locate(&self, addr: u64, len: usize, access: poly_a64::Access) -> Result<(bool, usize), poly_a64::MemFault> {
        let fail = poly_a64::MemFault { addr, access, alignment: false };
        let end = addr.checked_add(len as u64).ok_or(fail)?;
        if addr >= self.data_base && end <= self.data_base + self.data.len() as u64 {
            return Ok((true, (addr - self.data_base) as usize));
        }
        if addr >= self.code_base && end <= self.code_base + self.code.len() as u64 {
            return Ok((false, (addr - self.code_base) as usize));
        }
        Err(fail)
    }
}

impl poly_a64::Memory for TwoRegions<'_> {
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), poly_a64::MemFault> {
        let (data, o) = self.locate(addr, buf.len(), poly_a64::Access::Read)?;
        let src = if data { &self.data[o..o + buf.len()] } else { &self.code[o..o + buf.len()] };
        buf.copy_from_slice(src);
        Ok(())
    }
    fn write(&mut self, addr: u64, src: &[u8]) -> Result<(), poly_a64::MemFault> {
        match self.locate(addr, src.len(), poly_a64::Access::Write)? {
            (true, o) => {
                self.data[o..o + src.len()].copy_from_slice(src);
                Ok(())
            }
            (false, _) => Err(poly_a64::MemFault { addr, access: poly_a64::Access::Write, alignment: false }),
        }
    }
}

fn record(tag: u32, cpu_x: &[u64; 31], sp: u64, nzcv: u64, fpsr: u64, v: &[u128; 32], data: &[u8]) -> Vec<u64> {
    let mut r = vec![0u64; REC_WORDS];
    r[0] = u64::from(tag);
    r[1..32].copy_from_slice(cpu_x);
    r[fuzz::REC_SP] = sp;
    r[fuzz::REC_NZCV] = nzcv;
    r[fuzz::REC_FPSR] = fpsr;
    for (i, q) in v.iter().enumerate() {
        r[fuzz::REC_V0 + 2 * i] = *q as u64;
        r[fuzz::REC_V0 + 2 * i + 1] = (*q >> 64) as u64;
    }
    for (i, c) in data[WIN_OFF..WIN_OFF + 512].chunks_exact(8).enumerate() {
        r[REC_WIN + i] = u64::from_le_bytes(c.try_into().unwrap());
    }
    r
}

/// Run one case on poly-a64.
#[allow(clippy::too_many_arguments)]
fn run_poly(
    tag: u32,
    st: &State,
    code_base: u64,
    code: &[u8],
    data_base: u64,
    data0: &[u8],
    start: u64,
    end: u64,
    host: &HostRegs,
) -> Run {
    let mut cpu = poly_a64::Cpu::new();
    cpu.x = st.x;
    cpu.sp = st.sp;
    cpu.nzcv = st.nzcv as u32;
    cpu.fpcr = (st.fpcr as u32) & 0x07C8_0000;
    cpu.v = st.v;
    cpu.pc = start;
    cpu.tpidrro_el0 = host.tpidrro;
    cpu.dczid = host.dczid;
    let mut mem = TwoRegions { code_base, code, data_base, data: data0.to_vec() };
    let mut how = End::Other(start);
    for _ in 0..64 {
        if cpu.pc == end {
            how = End::Done;
            break;
        }
        match cpu.step(&mut mem) {
            poly_a64::Outcome::Continue => {}
            poly_a64::Outcome::Fault(_) => {
                how = End::Fault(cpu.pc);
                break;
            }
            poly_a64::Outcome::Undefined(_) => {
                how = End::Undefined(cpu.pc);
                break;
            }
            poly_a64::Outcome::Unsupported(w) => {
                how = End::Unsupported(cpu.pc, w);
                break;
            }
            _ => {
                how = End::Other(cpu.pc);
                break;
            }
        }
    }
    let rec = record(tag, &cpu.x, cpu.sp, u64::from(cpu.nzcv), u64::from(cpu.fpsr), &cpu.v, &mem.data);
    Run { rec, end: how, data: mem.data }
}

/// What the host tells poly-a64 about itself.
pub struct HostRegs {
    pub tpidrro: u64,
    pub dczid: u64,
}

pub fn run(opts: &Opts) -> Result<bool, String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        host::run(opts)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = opts;
        println!("a64diff native needs an arm64 Mac (the host CPU is the oracle); skipping on this host.");
        Ok(true)
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod host {
    use super::*;
    use core::cell::UnsafeCell;

    // The entry and exit of a case. `a64n_enter(state, code, host_sp_slot)` saves
    // the callee-saved registers and FPCR on the stack, stores the stack pointer
    // in the slot and branches to the case, whose prologue loads the state. The
    // signal handler resumes at `a64n_resume` with that stack pointer.
    core::arch::global_asm!(
        ".p2align 2",
        ".globl _a64n_enter",
        "_a64n_enter:",
        "sub sp, sp, #176",
        "stp x19, x20, [sp, #0]",
        "stp x21, x22, [sp, #16]",
        "stp x23, x24, [sp, #32]",
        "stp x25, x26, [sp, #48]",
        "stp x27, x28, [sp, #64]",
        "stp x29, x30, [sp, #80]",
        "stp d8, d9, [sp, #96]",
        "stp d10, d11, [sp, #112]",
        "stp d12, d13, [sp, #128]",
        "stp d14, d15, [sp, #144]",
        "mrs x3, fpcr",
        "str x3, [sp, #160]",
        "mov x3, sp",
        "str x3, [x2]",
        "br x1",
        ".globl _a64n_resume",
        "_a64n_resume:",
        "ldr x3, [sp, #160]",
        "msr fpcr, x3",
        "ldp x19, x20, [sp, #0]",
        "ldp x21, x22, [sp, #16]",
        "ldp x23, x24, [sp, #32]",
        "ldp x25, x26, [sp, #48]",
        "ldp x27, x28, [sp, #64]",
        "ldp x29, x30, [sp, #80]",
        "ldp d8, d9, [sp, #96]",
        "ldp d10, d11, [sp, #112]",
        "ldp d12, d13, [sp, #128]",
        "ldp d14, d15, [sp, #144]",
        "add sp, sp, #176",
        "ret",
        ".globl _a64n_host_regs",
        "_a64n_host_regs:",
        "mrs x1, tpidrro_el0",
        "str x1, [x0]",
        "mrs x1, dczid_el0",
        "str x1, [x0, #8]",
        "ret",
    );

    unsafe extern "C" {
        fn a64n_enter(state: *const u64, code: u64, host_sp: *mut u64);
        fn a64n_resume();
        fn a64n_host_regs(out: *mut u64);
        fn sys_icache_invalidate(start: *mut libc::c_void, len: libc::size_t);
        fn pthread_jit_write_protect_np(enabled: libc::c_int);
    }

    /// `struct __darwin_arm_thread_state64` (SDK `mach/arm/_structs.h`).
    #[repr(C)]
    struct ThreadState {
        x: [u64; 29],
        fp: u64,
        lr: u64,
        sp: u64,
        pc: u64,
        cpsr: u32,
        pad: u32,
    }

    /// `struct __darwin_mcontext64` (SDK `arm/_mcontext.h`): exception state
    /// (`far`, `esr`, `exception`), thread state, NEON state.
    #[repr(C)]
    struct MContext {
        far: u64,
        esr: u32,
        exception: u32,
        ss: ThreadState,
        v: [u128; 32],
        fpsr: u32,
        fpcr: u32,
    }

    /// What the handler saw.
    #[derive(Clone, Copy)]
    struct Caught {
        sig: i32,
        x: [u64; 31],
        sp: u64,
        pc: u64,
        cpsr: u64,
        v: [u128; 32],
        fpsr: u64,
    }

    struct Shared {
        host_sp: UnsafeCell<u64>,
        caught: UnsafeCell<Option<Caught>>,
    }
    // SAFETY: the tool is single-threaded; the cell is written by the signal
    // handler only while the main thread runs a case and read after it.
    unsafe impl Sync for Shared {}
    static SHARED: Shared = Shared { host_sp: UnsafeCell::new(0), caught: UnsafeCell::new(None) };

    extern "C" fn handler(sig: libc::c_int, _info: *mut libc::siginfo_t, uc: *mut libc::c_void) {
        // SAFETY: the kernel passes a valid `ucontext_t`; `uc_mcontext` is the
        // pointer at offset 48 (SDK `sys/_types/_ucontext.h`: int, sigset_t,
        // stack_t (24 bytes), uc_link, uc_mcsize, uc_mcontext).
        let mc = unsafe { &mut **(uc.cast::<u8>().add(48) as *mut *mut MContext) };
        let mut x = [0u64; 31];
        x[..29].copy_from_slice(&mc.ss.x);
        x[29] = mc.ss.fp;
        x[30] = mc.ss.lr;
        let c = Caught {
            sig,
            x,
            sp: mc.ss.sp,
            pc: mc.ss.pc,
            cpsr: u64::from(mc.ss.cpsr),
            v: mc.v,
            fpsr: u64::from(mc.fpsr),
        };
        // SAFETY: see `Shared`.
        unsafe {
            *SHARED.caught.get() = Some(c);
            mc.ss.sp = *SHARED.host_sp.get();
        }
        mc.ss.pc = a64n_resume as *const () as u64;
        mc.fpcr = 0;
    }

    fn install() {
        const ALT: usize = 256 * 1024;
        let stack = Box::leak(vec![0u8; ALT].into_boxed_slice());
        let ss = libc::stack_t { ss_sp: stack.as_mut_ptr().cast(), ss_size: ALT, ss_flags: 0 };
        // SAFETY: plain libc calls with valid arguments; the alternate stack is
        // leaked so it lives for the rest of the process.
        unsafe {
            libc::sigaltstack(&ss, core::ptr::null_mut());
            for sig in [libc::SIGTRAP, libc::SIGILL, libc::SIGSEGV, libc::SIGBUS, libc::SIGFPE] {
                let mut sa: libc::sigaction = core::mem::zeroed();
                sa.sa_sigaction = handler as *const () as usize;
                sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigaction(sig, &sa, core::ptr::null_mut());
            }
        }
    }

    /// An anonymous mapping that is unmapped on drop.
    struct Map {
        ptr: *mut u8,
        len: usize,
    }
    impl Map {
        fn new(len: usize, prot: libc::c_int, flags: libc::c_int) -> Result<Map, String> {
            // SAFETY: an anonymous private mapping; the result is checked.
            let p = unsafe { libc::mmap(core::ptr::null_mut(), len, prot, libc::MAP_PRIVATE | libc::MAP_ANON | flags, -1, 0) };
            if p == libc::MAP_FAILED {
                return Err(format!("mmap of {len} bytes failed"));
            }
            Ok(Map { ptr: p.cast(), len })
        }
        fn addr(&self) -> u64 {
            self.ptr as u64
        }
    }
    impl Drop for Map {
        fn drop(&mut self) {
            // SAFETY: unmaps exactly what `new` mapped.
            unsafe { libc::munmap(self.ptr.cast(), self.len) };
        }
    }

    pub fn run(opts: &Opts) -> Result<bool, String> {
        let fams: Vec<&fuzz::Family> =
            FAMILIES.iter().filter(|f| opts.family.as_ref().is_none_or(|n| n == f.name)).collect();
        if fams.is_empty() {
            return Err("no such family (see `a64diff list`)".into());
        }
        install();
        let mut hr = [0u64; 2];
        // SAFETY: writes two words to `hr`.
        unsafe { a64n_host_regs(hr.as_mut_ptr()) };
        let host = HostRegs { tpidrro: hr[0], dczid: hr[1] };

        let work = std::env::temp_dir().join(format!("a64native-{}", std::process::id()));
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        let mut rng = Rng(opts.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);

        // Data region with guard pages on both sides.
        let page = 16 * 1024;
        let guard = 1 << 20;
        let data_map = Map::new(DATA + 2 * guard, libc::PROT_NONE, 0)?;
        let data_ptr = data_map.addr() + guard as u64;
        // SAFETY: the middle of the reservation becomes readable and writable.
        unsafe { libc::mprotect(data_ptr as *mut libc::c_void, DATA, libc::PROT_READ | libc::PROT_WRITE) };

        let mut per_family: BTreeMap<&str, [usize; 4]> = BTreeMap::new(); // pass, fail, unsupported, skipped
        let mut reported = 0;
        let mut unsupported: Vec<u32> = Vec::new();
        let mut mismatches: Vec<String> = Vec::new();
        let mut ok = true;
        let mut done = 0;
        let mut batch_no = 0;
        while done < opts.blocks {
            let n = (opts.blocks - done).min(2000);
            let mut cases = Vec::with_capacity(n);
            for _ in 0..n {
                let f = fams[rng.below(fams.len() as u32) as usize];
                cases.push(((f.make)(&mut rng), State::random(&mut rng)));
            }
            let asm = assembly(&cases);
            let s_path = work.join(format!("batch{batch_no}.s"));
            let o_path = work.join(format!("batch{batch_no}.o"));
            std::fs::write(&s_path, &asm).map_err(|e| e.to_string())?;
            assemble(&s_path, &o_path)?;
            let obj = std::fs::read(&o_path).map_err(|e| e.to_string())?;
            let (text, syms) = read_object(&obj)?;
            if let Some(dir) = &opts.keep {
                let _ = std::fs::create_dir_all(dir);
                let _ = std::fs::copy(&s_path, dir.join(format!("native{batch_no}.s")));
            }

            let code_len = text.len().next_multiple_of(page) + page;
            let code_map = Map::new(code_len, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC, libc::MAP_JIT)?;
            // SAFETY: the mapping is `code_len` bytes, at least `text.len()`; JIT
            // write protection is lifted for this thread while copying.
            unsafe {
                pthread_jit_write_protect_np(0);
                core::ptr::copy_nonoverlapping(text.as_ptr(), code_map.ptr, text.len());
                pthread_jit_write_protect_np(1);
                sys_icache_invalidate(code_map.ptr.cast(), text.len());
            }
            let base = code_map.addr();

            for (i, (case, st)) in cases.iter_mut().enumerate() {
                let sym = |p: &str| syms.get(&format!("_{p}{i}")).copied().ok_or(format!("symbol _{p}{i} missing"));
                let (c, s, e) = (sym("c")?, sym("s")?, sym("e")?);
                // Initial data: random bytes, the state's window around x5.
                let mut data0 = vec![0u8; DATA];
                for ch in data0.chunks_mut(8) {
                    ch.copy_from_slice(&rng.next().to_le_bytes());
                }
                data0[WIN_OFF..WIN_OFF + 512].copy_from_slice(&st.window);
                if case.base5 {
                    st.x[5] = data_ptr + (DATA / 2) as u64;
                }
                if case.family == "rand-ldst" {
                    // Register offsets and post-index registers stay within the
                    // data region and its guard pages.
                    for (r, v) in st.x.iter_mut().enumerate() {
                        if r != 5 {
                            *v &= 0x7FF;
                        }
                    }
                }
                // The OS may zero x18 (the platform register) at any time.
                st.x[18] = 0;
                let words = st.words();
                let poly = run_poly(i as u32, st, base, &text, data_ptr, &data0, base + s, base + e, &host);
                // Host run, repeated (up to three times) if it differs: a
                // store-exclusive may fail spuriously on real hardware.
                let mut host_run = None;
                for _attempt in 0..3 {
                    // Host run.
                    // SAFETY: `data_ptr` is DATA writable bytes.
                    unsafe { core::ptr::copy_nonoverlapping(data0.as_ptr(), data_ptr as *mut u8, DATA) };
                    // SAFETY: see `Shared`; no case is running.
                    unsafe { *SHARED.caught.get() = None };
                    // SAFETY: the case code loads `words` and ends in a BRK or a
                    // fault, which the handler turns into a return to here.
                    unsafe { a64n_enter(words.as_ptr(), base + c, SHARED.host_sp.get()) };
                    // SAFETY: the handler has run; no case is running.
                    let caught = unsafe { (*SHARED.caught.get()).ok_or("the case returned without a signal")? };
                    let mut hdata = vec![0u8; DATA];
                    // SAFETY: reads DATA bytes of the data region.
                    unsafe { core::ptr::copy_nonoverlapping(data_ptr as *const u8, hdata.as_mut_ptr(), DATA) };
                    let hend = match caught.sig {
                        libc::SIGTRAP if caught.pc == base + e => End::Done,
                        libc::SIGILL => End::Undefined(caught.pc),
                        libc::SIGSEGV | libc::SIGBUS => End::Fault(caught.pc),
                        _ => End::Other(caught.pc),
                    };
                    let hrec = record(
                        i as u32,
                        &caught.x,
                        caught.sp,
                        caught.cpsr & 0xF000_0000,
                        caught.fpsr,
                        &caught.v,
                        &hdata,
                    );
                    let run = Run { rec: hrec, end: hend, data: hdata };
                    let same = run.end == poly.end
                        && run.data == poly.data
                        && (0..REC_WORDS).all(|k| k == 1 + 18 || run.rec[k] == poly.rec[k]);
                    host_run = Some(run);
                    if same {
                        break;
                    }
                }
                let host_run = host_run.expect("ran at least once");

                let fam = case.family;
                let entry = per_family.entry(fam).or_default();
                if let End::Unsupported(_, w) = poly.end {
                    if let End::Undefined(_) = host_run.end {
                        // Both stop: the host has no such instruction either.
                        entry[3] += 1;
                    } else {
                        entry[2] += 1;
                        unsupported.push(w);
                        if entry[2] <= opts.verbose {
                            println!("unsupported [{fam}] {}", case.body.join(" ; "));
                        }
                    }
                    continue;
                }
                let mut diffs = Vec::new();
                if host_run.end != poly.end {
                    diffs.push(format!("end  host={:?} poly={:?} (base {base:#x})", rel(host_run.end, base), rel(poly.end, base)));
                }
                let mut d = crate::diff_record(&host_run.rec, &poly.rec);
                d.retain(|l| !l.starts_with("x18 "));
                // PACGA's code depends on the host's secret key.
                if let Some(w) = crate::word_of(case).filter(|w| w & 0xFFE0_FC00 == 0x9AC0_3000) {
                    let rd = format!("x{:<2} ", w & 31);
                    d.retain(|l| !l.starts_with(&rd));
                }
                diffs.extend(d.into_iter().map(|l| l.replace("qemu=", "host=")));
                if host_run.data != poly.data {
                    let first = host_run.data.iter().zip(&poly.data).position(|(a, b)| a != b).unwrap_or(0);
                    if !(WIN_OFF..WIN_OFF + 512).contains(&first) {
                        diffs.push(format!("data differs outside the window at x5{:+}", first as i64 - (DATA / 2) as i64));
                    }
                }
                if diffs.is_empty() {
                    entry[0] += 1;
                } else {
                    entry[1] += 1;
                    ok = false;
                    let kind = |e: End| match e {
                        End::Done => "done",
                        End::Fault(_) => "fault",
                        End::Undefined(_) => "undefined",
                        End::Unsupported(..) => "unsupported",
                        End::Other(_) => "other",
                    };
                    if let Some(w) = crate::word_of(case) {
                        mismatches.push(format!("{w:#010x}\t{}\t{}\t{fam}", kind(host_run.end), kind(poly.end)));
                    }
                    if reported < 25 + opts.verbose {
                        reported += 1;
                        println!("MISMATCH [{fam}] {}", case.body.join(" ; "));
                        if !case.setup.is_empty() {
                            println!("  setup: {}", case.setup.join(" ; "));
                        }
                        for l in diffs.iter().take(12) {
                            println!("  {l}");
                        }
                    }
                }
            }
            done += n;
            batch_no += 1;
        }
        let _ = std::fs::remove_dir_all(&work);
        if let Some(dir) = &opts.keep {
            // The words the host ran and poly-a64 does not implement, one per line.
            let text: String = unsupported.iter().map(|w| format!(".inst {w:#010x}\n")).collect();
            let _ = std::fs::write(dir.join("unsupported.s"), text);
            // Mismatching single-word cases: word, host end, poly-a64 end, family.
            let _ = std::fs::write(dir.join("mismatch.tsv"), mismatches.join("\n") + "\n");
        }
        println!();
        println!("{:<18} {:>7} {:>6} {:>8} {:>8}", "family", "pass", "fail", "unsupp", "both-ud");
        let mut t = [0usize; 4];
        for (fam, c) in &per_family {
            println!("{fam:<18} {:>7} {:>6} {:>8} {:>8}", c[0], c[1], c[2], c[3]);
            for k in 0..4 {
                t[k] += c[k];
            }
        }
        println!(
            "{:<18} {:>7} {:>6} {:>8} {:>8}  (host CPU oracle, seed {})",
            "total", t[0], t[1], t[2], t[3], opts.seed
        );
        // `unsupp` (the host ran it, poly-a64 does not implement it) is a
        // coverage gap, not a wrong result; it fails the run only for the
        // hand-made families, which only make encodings poly-a64 covers.
        let gaps = per_family.iter().any(|(f, c)| !f.starts_with("rand-") && c[2] > 0);
        ok &= !gaps;
        Ok(ok)
    }

    fn rel(e: End, base: u64) -> String {
        match e {
            End::Done => "done".into(),
            End::Fault(p) => format!("fault@+{:#x}", p.wrapping_sub(base)),
            End::Undefined(p) => format!("undefined@+{:#x}", p.wrapping_sub(base)),
            End::Unsupported(p, w) => format!("unsupported {w:#010x}@+{:#x}", p.wrapping_sub(base)),
            End::Other(p) => format!("stopped@+{:#x}", p.wrapping_sub(base)),
        }
    }

    fn assemble(s: &Path, o: &Path) -> Result<(), String> {
        let out = Command::new(std::env::var("A64_CC").unwrap_or_else(|_| "clang".into()))
            .args(["-c", "-arch", "arm64", "-x", "assembler"])
            .arg(s)
            .arg("-o")
            .arg(o)
            .output()
            .map_err(|e| format!("clang: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("clang failed:\n{}", err.lines().take(20).collect::<Vec<_>>().join("\n")));
        }
        Ok(())
    }
}

#[allow(dead_code)]
const _: () = assert!(REC_BYTES == REC_WORDS * 8);
