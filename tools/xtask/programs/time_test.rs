//! `/bin/time-test`: the time system calls and the vDSO from ring 3 (ADR
//! 0044), a static Linux program without any C library that `cargo xtask`
//! builds like the nolibc hello (`rustc --target x86_64-unknown-none`,
//! static, not PIE; see `tools/xtask/src/programs.rs`).
//!
//! It finds the vDSO through `AT_SYSINFO_EHDR`, resolves its functions by
//! walking the image's dynamic symbol table (`DT_HASH`, `DT_SYMTAB`,
//! `DT_STRTAB`) and checking each symbol's version (`DT_VERSYM`,
//! `DT_VERDEF`) is `LINUX_2.6`, as vdso(7) describes, and calls them. Then
//! it checks the clocks and the vDSO against the system calls, sleeps,
//! timers delivering signals to handlers, and CPU-time clocks.
//!
//! Every check prints `time-test: ok <name>` or `time-test: FAIL <name>:
//! <value>`; the program ends with `time-test: done, <n> failed` and exit
//! status `n`. Time bounds are generous ("at least", never a tight "at
//! most") so a loaded host does not fail it.
//!
//! Numbers and layouts from the x86-64 system call table and the UAPI
//! headers (`linux/time.h`, `asm-generic/siginfo.h`, `asm/signal.h`,
//! `linux/auxvec.h`), the ELF gABI and vdso(7). Written for Poly.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU64, Ordering::SeqCst};

const SYS_WRITE: u64 = 1;
const SYS_RT_SIGACTION: u64 = 13;
const SYS_NANOSLEEP: u64 = 35;
const SYS_ALARM: u64 = 37;
const SYS_SETITIMER: u64 = 38;
const SYS_GETITIMER: u64 = 36;
const SYS_GETTIMEOFDAY: u64 = 96;
const SYS_GETRUSAGE: u64 = 98;
const SYS_TIMES: u64 = 100;
const SYS_TIME: u64 = 201;
const SYS_TIMER_CREATE: u64 = 222;
const SYS_TIMER_SETTIME: u64 = 223;
const SYS_TIMER_GETTIME: u64 = 224;
const SYS_TIMER_DELETE: u64 = 226;
const SYS_CLOCK_SETTIME: u64 = 227;
const SYS_CLOCK_GETTIME: u64 = 228;
const SYS_CLOCK_GETRES: u64 = 229;
const SYS_CLOCK_NANOSLEEP: u64 = 230;
const SYS_EXIT_GROUP: u64 = 231;

const CLOCK_REALTIME: u64 = 0;
const CLOCK_MONOTONIC: u64 = 1;
const CLOCK_PROCESS_CPUTIME_ID: u64 = 2;
const CLOCK_THREAD_CPUTIME_ID: u64 = 3;
const CLOCK_MONOTONIC_COARSE: u64 = 6;
const TIMER_ABSTIME: u64 = 1;
const ITIMER_REAL: u64 = 0;
const ITIMER_VIRTUAL: u64 = 1;
const SIGALRM: u64 = 14;
const SIGUSR1: u64 = 10;
const SIGVTALRM: u64 = 26;
const SA_SIGINFO: u64 = 0x4;
const SA_RESTORER: u64 = 0x0400_0000;
const SIGEV_SIGNAL: i32 = 0;
const SI_TIMER: i32 = -2;
const EINTR: i64 = 4;
const AT_NULL: u64 = 0;
const AT_SYSINFO_EHDR: u64 = 33;

const MS: u64 = 1_000_000;
const SEC: u64 = 1_000_000_000;

/// # Safety
/// The call must not break the program's memory.
unsafe fn syscall(nr: u64, a: [u64; 6]) -> i64 {
    let ret;
    // SAFETY: the x86-64 Linux convention: rcx and r11 are clobbered,
    // memory may be read and written by the kernel.
    unsafe {
        asm!("syscall", inlateout("rax") nr => ret, in("rdi") a[0], in("rsi") a[1], in("rdx") a[2],
             in("r10") a[3], in("r8") a[4], in("r9") a[5], lateout("rcx") _, lateout("r11") _,
             options(nostack));
    }
    ret
}

fn sys(nr: u64, a: &[u64]) -> i64 {
    let mut args = [0u64; 6];
    args[..a.len()].copy_from_slice(a);
    // SAFETY: every call below passes pointers to this program's own
    // variables, sized as the kernel expects.
    unsafe { syscall(nr, args) }
}

fn write(s: &[u8]) {
    sys(SYS_WRITE, &[1, s.as_ptr() as u64, s.len() as u64]);
}

/// A signed number in decimal.
fn write_num(n: i64) {
    let mut buf = [0u8; 24];
    let mut i = buf.len();
    let mut v = n.unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if n < 0 {
        i -= 1;
        buf[i] = b'-';
    }
    write(&buf[i..]);
}

static FAILED: AtomicU64 = AtomicU64::new(0);

/// Report one check: `value` is printed when it fails.
fn check(name: &[u8], ok: bool, value: i64) {
    if ok {
        write(b"time-test: ok ");
        write(name);
    } else {
        FAILED.fetch_add(1, SeqCst);
        write(b"time-test: FAIL ");
        write(name);
        write(b": ");
        write_num(value);
    }
    write(b"\n");
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Timespec {
    sec: i64,
    nsec: i64,
}

impl Timespec {
    fn ns(self) -> u64 {
        self.sec as u64 * SEC + self.nsec as u64
    }

    fn from_ns(ns: u64) -> Timespec {
        Timespec { sec: (ns / SEC) as i64, nsec: (ns % SEC) as i64 }
    }
}

fn clock(id: u64) -> u64 {
    let mut t = Timespec::default();
    let r = sys(SYS_CLOCK_GETTIME, &[id, &raw mut t as u64]);
    if r != 0 {
        check(b"clock_gettime", false, r);
    }
    t.ns()
}

fn mono() -> u64 {
    clock(CLOCK_MONOTONIC)
}

// ---- the vDSO ----------------------------------------------------------------

type ClockGettime = extern "C" fn(i32, *mut Timespec) -> i64;
type ClockGetres = extern "C" fn(i32, *mut Timespec) -> i64;
type Gettimeofday = extern "C" fn(*mut [i64; 2], *mut [i32; 2]) -> i64;
type Time = extern "C" fn(*mut i64) -> i64;
type Getcpu = extern "C" fn(*mut u32, *mut u32, *mut u8) -> i64;

/// Read a `T` at `addr` in the vDSO image.
///
/// # Safety
/// `addr` must lie in the mapped image.
unsafe fn at<T: Copy>(addr: usize) -> T {
    // SAFETY: the caller's promise; unaligned reads are fine.
    unsafe { (addr as *const T).read_unaligned() }
}

/// The NUL-terminated string at `addr`.
///
/// # Safety
/// `addr` must point at a NUL-terminated string in the image.
unsafe fn cstr(addr: usize) -> &'static [u8] {
    let mut len = 0;
    // SAFETY: the caller's promise.
    while unsafe { at::<u8>(addr + len) } != 0 {
        len += 1;
    }
    // SAFETY: as above, `len` bytes before the NUL.
    unsafe { core::slice::from_raw_parts(addr as *const u8, len) }
}

/// The dynamic symbol table of the vDSO at `base` (its ELF header).
struct Vdso {
    base: usize,
    strtab: usize,
    symtab: usize,
    count: usize,
    versym: usize,
    verdef: usize,
}

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_NULL: i64 = 0;
const DT_HASH: i64 = 4;
const DT_STRTAB: i64 = 5;
const DT_SYMTAB: i64 = 6;
const DT_VERSYM: i64 = 0x6fff_fff0;
const DT_VERDEF: i64 = 0x6fff_fffc;

impl Vdso {
    /// Parse the image at `base`; `Err(step)` names what is missing.
    ///
    /// # Safety
    /// `base` must be the vDSO's ELF header as `AT_SYSINFO_EHDR` gives it.
    unsafe fn parse(base: usize) -> Result<Vdso, i64> {
        // SAFETY (the reads below): offsets from the ELF header and its
        // dynamic section, which the kernel mapped (the whole image).
        unsafe {
            if at::<[u8; 4]>(base) != *b"\x7fELF" {
                return Err(1);
            }
            let (phoff, phnum) = (at::<u64>(base + 32) as usize, at::<u16>(base + 56) as usize);
            let (mut bias, mut dynamic) = (None, None);
            for i in 0..phnum {
                let ph = base + phoff + i * 56;
                let vaddr = at::<u64>(ph + 16) as usize;
                match at::<u32>(ph) {
                    PT_LOAD if bias.is_none() => bias = Some(base - vaddr),
                    PT_DYNAMIC => dynamic = Some(vaddr),
                    _ => {}
                }
            }
            let bias = bias.ok_or(2)?;
            let mut d = bias + dynamic.ok_or(3)?;
            let mut v = Vdso { base, strtab: 0, symtab: 0, count: 0, versym: 0, verdef: 0 };
            loop {
                let (tag, val) = (at::<i64>(d), at::<u64>(d + 8) as usize);
                match tag {
                    DT_NULL => break,
                    // nchain: the number of symbols.
                    DT_HASH => v.count = at::<u32>(bias + val + 4) as usize,
                    DT_STRTAB => v.strtab = bias + val,
                    DT_SYMTAB => v.symtab = bias + val,
                    DT_VERSYM => v.versym = bias + val,
                    DT_VERDEF => v.verdef = bias + val,
                    _ => {}
                }
                d += 16;
            }
            if v.strtab == 0 || v.symtab == 0 || v.count == 0 {
                return Err(4);
            }
            Ok(v)
        }
    }

    /// The name of version `index` in the version definitions.
    fn version_name(&self, index: u16) -> Option<&'static [u8]> {
        if self.verdef == 0 {
            return None;
        }
        let mut def = self.verdef;
        // SAFETY: the verdef chain of the image (vd_ndx at 4, vd_aux at 12,
        // vd_next at 16; vda_name at 0 of the first aux entry).
        unsafe {
            loop {
                if at::<u16>(def + 4) == index {
                    let aux = def + at::<u32>(def + 12) as usize;
                    return Some(cstr(self.strtab + at::<u32>(aux) as usize));
                }
                let next = at::<u32>(def + 16) as usize;
                if next == 0 {
                    return None;
                }
                def += next;
            }
        }
    }

    /// The address of the defined function `name` in version `LINUX_2.6`.
    fn lookup(&self, name: &[u8]) -> Option<usize> {
        for i in 1..self.count {
            let sym = self.symtab + i * 24;
            // SAFETY: symbol `i` of the table (st_name at 0, st_info at 4,
            // st_shndx at 6, st_value at 8) and its name.
            let (sym_name, info, shndx, value) = unsafe {
                (cstr(self.strtab + at::<u32>(sym) as usize), at::<u8>(sym + 4), at::<u16>(sym + 6), at::<u64>(sym + 8))
            };
            const STT_FUNC: u8 = 2;
            if sym_name != name || info & 0xf != STT_FUNC || shndx == 0 {
                continue;
            }
            if self.versym != 0 {
                // SAFETY: entry `i` of the version table.
                let version = unsafe { at::<u16>(self.versym + i * 2) } & 0x7fff;
                if self.version_name(version) != Some(b"LINUX_2.6".as_slice()) {
                    continue;
                }
            }
            return Some(self.base + value as usize);
        }
        None
    }
}

/// `AT_SYSINFO_EHDR` from the auxiliary vector above the initial stack.
///
/// # Safety
/// `sp` must be the initial stack pointer (pointing at `argc`).
unsafe fn sysinfo_ehdr(sp: *const u64) -> u64 {
    // SAFETY: the psABI initial stack: argc, argv[] and NULL, envp[] and
    // NULL, then the auxiliary vector up to AT_NULL.
    unsafe {
        let argc = *sp as usize;
        let mut p = sp.add(1 + argc + 1);
        while *p != 0 {
            p = p.add(1);
        }
        p = p.add(1);
        loop {
            let (ty, val) = (*p, *p.add(1));
            if ty == AT_NULL {
                return 0;
            }
            if ty == AT_SYSINFO_EHDR {
                return val;
            }
            p = p.add(2);
        }
    }
}

fn vdso_tests(sp: *const u64) {
    // SAFETY: `sp` is the initial stack pointer `_start` passed on.
    let ehdr = unsafe { sysinfo_ehdr(sp) };
    check(b"vdso_announced_in_auxv", ehdr != 0 && ehdr % 4096 == 0, ehdr as i64);
    if ehdr == 0 {
        return;
    }
    // SAFETY: the kernel mapped the vDSO image there.
    let vdso = match unsafe { Vdso::parse(ehdr as usize) } {
        Ok(v) => v,
        Err(step) => return check(b"vdso_parses", false, step),
    };
    check(b"vdso_parses", true, 0);
    let names: [&[u8]; 5] =
        [b"__vdso_clock_gettime", b"__vdso_gettimeofday", b"__vdso_time", b"__vdso_clock_getres", b"__vdso_getcpu"];
    let mut addrs = [0usize; 5];
    for (i, name) in names.iter().enumerate() {
        addrs[i] = vdso.lookup(name).unwrap_or(0);
    }
    let missing = addrs.iter().position(|&a| a == 0);
    check(b"vdso_exports_its_functions_in_linux_2_6", missing.is_none(), missing.map_or(0, |i| i as i64));
    if missing.is_some() {
        return;
    }
    // SAFETY: the addresses of the vDSO's functions with these C
    // signatures (vdso(7)).
    let (gettime, gettimeofday, time, getres, getcpu) = unsafe {
        (
            core::mem::transmute::<usize, ClockGettime>(addrs[0]),
            core::mem::transmute::<usize, Gettimeofday>(addrs[1]),
            core::mem::transmute::<usize, Time>(addrs[2]),
            core::mem::transmute::<usize, ClockGetres>(addrs[3]),
            core::mem::transmute::<usize, Getcpu>(addrs[4]),
        )
    };

    // clock_gettime: between two system calls (within a tolerance: the
    // vDSO may compute from the TSC while the kernel reads another
    // counter), and never backwards.
    const TOLERANCE: u64 = 50 * MS;
    for (id, name) in [
        (CLOCK_MONOTONIC, b"vdso_monotonic_agrees_with_the_syscall".as_slice()),
        (CLOCK_REALTIME, b"vdso_realtime_agrees_with_the_syscall"),
    ] {
        let mut worst = 0i64;
        let mut ok = true;
        for _ in 0..200 {
            let before = clock(id);
            let mut t = Timespec::default();
            let r = gettime(id as i32, &mut t);
            let after = clock(id);
            let v = t.ns();
            ok &= r == 0 && t.nsec >= 0 && t.nsec < SEC as i64;
            ok &= v + TOLERANCE >= before && v <= after + TOLERANCE;
            let off = if v < before {
                (before - v) as i64
            } else if v > after {
                (v - after) as i64
            } else {
                0
            };
            worst = worst.max(off);
        }
        check(name, ok, worst);
    }
    let mut last = 0;
    let mut backwards = 0;
    for _ in 0..20_000 {
        let mut t = Timespec::default();
        gettime(CLOCK_MONOTONIC as i32, &mut t);
        if t.ns() < last {
            backwards += 1;
        }
        last = t.ns();
    }
    check(b"vdso_monotonic_never_goes_backwards", backwards == 0, backwards);
    // Clocks the vDSO does not compute go to the system call.
    let mut t = Timespec::default();
    let r = gettime(CLOCK_PROCESS_CPUTIME_ID as i32, &mut t);
    check(b"vdso_cpu_clock_through_the_syscall", r == 0 && t.ns() > 0, r);
    let r = gettime(12345, &mut t);
    check(b"vdso_bad_clock_is_einval", r == -22, r);
    let r = gettime(CLOCK_MONOTONIC as i32, core::ptr::null_mut());
    check(b"vdso_null_pointer_is_efault", r == -14, r);

    // gettimeofday and time against clock_gettime(CLOCK_REALTIME).
    let real = clock(CLOCK_REALTIME);
    let mut tv = [0i64; 2];
    let mut tz = [-1i32; 2];
    let r = gettimeofday(&mut tv, &mut tz);
    let ok =
        r == 0 && tv[1] >= 0 && tv[1] < 1_000_000 && (tv[0] as u64 * SEC + tv[1] as u64 * 1000) + TOLERANCE >= real;
    check(b"vdso_gettimeofday", ok, r);
    let mut sys_tz = [-1i32; 2];
    sys(SYS_GETTIMEOFDAY, &[0, &raw mut sys_tz as u64]);
    check(b"vdso_gettimeofday_timezone", tz == sys_tz, tz[0] as i64);
    let mut stored = 0i64;
    let secs = time(&mut stored);
    let sys_secs = sys(SYS_TIME, &[0]);
    check(b"vdso_time", secs == stored && secs.abs_diff(sys_secs) <= 1 && secs > 1_577_836_800, secs);
    // clock_getres: the same answers as the system call.
    let mut ok = true;
    for id in [CLOCK_REALTIME, CLOCK_MONOTONIC, CLOCK_MONOTONIC_COARSE, CLOCK_THREAD_CPUTIME_ID] {
        let (mut a, mut b) = (Timespec { sec: -1, nsec: -1 }, Timespec::default());
        let ra = getres(id as i32, &mut a);
        let rb = sys(SYS_CLOCK_GETRES, &[id, &raw mut b as u64]);
        ok &= ra == 0 && rb == 0 && a.ns() == b.ns();
    }
    let mut res = Timespec::default();
    getres(CLOCK_MONOTONIC as i32, &mut res);
    check(b"vdso_clock_getres", ok && res.ns() == 1, res.ns() as i64);
    // The CPU it runs on (any of them: it may move), node 0.
    let (mut cpu, mut node) = (u32::MAX, 7u32);
    let r = getcpu(&mut cpu, &mut node, core::ptr::null_mut());
    check(b"vdso_getcpu", r == 0 && cpu < 64 && node == 0, r);

    // Setting the wall clock moves the vDSO's wall clock too, not the
    // monotonic clock; then it is put back.
    let mono0 = mono();
    let real0 = clock(CLOCK_REALTIME);
    let ahead = Timespec::from_ns(real0 + 1000 * SEC);
    let r = sys(SYS_CLOCK_SETTIME, &[CLOCK_REALTIME, &raw const ahead as u64]);
    let mut t = Timespec::default();
    gettime(CLOCK_REALTIME as i32, &mut t);
    let moved = t.ns().saturating_sub(real0);
    let mono1 = mono();
    let back = Timespec::from_ns(clock(CLOCK_REALTIME) - 1000 * SEC);
    let r2 = sys(SYS_CLOCK_SETTIME, &[CLOCK_REALTIME, &raw const back as u64]);
    let ok = r == 0 && r2 == 0 && moved >= 1000 * SEC && moved < 1000 * SEC + 10 * SEC && mono1 - mono0 < 10 * SEC;
    check(b"clock_settime_moves_the_wall_clock_only", ok, moved as i64);
    let below = Timespec { sec: 0, nsec: 1 };
    let r = sys(SYS_CLOCK_SETTIME, &[CLOCK_REALTIME, &raw const below as u64]);
    check(b"clock_settime_below_monotonic_is_einval", r == -22, r);
    let r = sys(SYS_CLOCK_SETTIME, &[CLOCK_MONOTONIC, &raw const ahead as u64]);
    check(b"clock_settime_monotonic_is_einval", r == -22, r);
}

// ---- sleeping ------------------------------------------------------------------

fn sleep_tests() {
    // nanosleep sleeps at least as long as asked.
    let req = Timespec::from_ns(20 * MS);
    let t0 = mono();
    let r = sys(SYS_NANOSLEEP, &[&raw const req as u64, 0]);
    let slept = mono() - t0;
    check(b"nanosleep_sleeps_at_least_the_time_asked", r == 0 && slept >= 20 * MS, slept as i64);
    let bad = Timespec { sec: 0, nsec: 1_000_000_000 };
    let r = sys(SYS_NANOSLEEP, &[&raw const bad as u64, 0]);
    check(b"nanosleep_bad_time_is_einval", r == -22, r);

    // clock_nanosleep with TIMER_ABSTIME, on the monotonic and the wall
    // clock.
    for (id, name) in [
        (CLOCK_MONOTONIC, b"clock_nanosleep_abstime_monotonic".as_slice()),
        (CLOCK_REALTIME, b"clock_nanosleep_abstime_realtime"),
    ] {
        let deadline = clock(id) + 30 * MS;
        let at = Timespec::from_ns(deadline);
        let r = sys(SYS_CLOCK_NANOSLEEP, &[id, TIMER_ABSTIME, &raw const at as u64, 0]);
        let now = clock(id);
        check(name, r == 0 && now >= deadline, now as i64 - deadline as i64);
    }
    // A deadline in the past returns at once.
    let past = Timespec::from_ns(mono() - 5 * MS);
    let r = sys(SYS_CLOCK_NANOSLEEP, &[CLOCK_MONOTONIC, TIMER_ABSTIME, &raw const past as u64, 0]);
    check(b"clock_nanosleep_past_deadline", r == 0, r);
    // Relative.
    let rel = Timespec::from_ns(10 * MS);
    let t0 = mono();
    let r = sys(SYS_CLOCK_NANOSLEEP, &[CLOCK_MONOTONIC, 0, &raw const rel as u64, 0]);
    check(b"clock_nanosleep_relative", r == 0 && mono() - t0 >= 10 * MS, r);
    let r = sys(SYS_CLOCK_NANOSLEEP, &[CLOCK_THREAD_CPUTIME_ID, 0, &raw const rel as u64, 0]);
    // The thread CPU clock cannot be slept on: EOPNOTSUPP (the spec of
    // clock_nanosleep).
    check(b"clock_nanosleep_thread_cpu_clock_is_eopnotsupp", r == -95, r);
}

// ---- signals from timers -----------------------------------------------------------

/// `struct sigaction` as the kernel takes it.
#[repr(C)]
struct SigAction {
    handler: u64,
    flags: u64,
    restorer: u64,
    mask: u64,
}

static ALARMS: AtomicU64 = AtomicU64::new(0);
static VTALARMS: AtomicU64 = AtomicU64::new(0);
static TIMER_SIGNALS: AtomicU64 = AtomicU64::new(0);
/// si_code, timer id and si_value of the last SIGUSR1.
static TIMER_INFO: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

extern "C" fn on_alarm(_: i32) {
    ALARMS.fetch_add(1, SeqCst);
}

extern "C" fn on_vtalarm(_: i32) {
    VTALARMS.fetch_add(1, SeqCst);
}

extern "C" fn on_timer(_: i32, info: *const u8, _: *mut u8) {
    // SAFETY: the kernel passes the siginfo it built (si_code at 8; the
    // `_timer` fields: si_tid at 16, si_value at 24).
    unsafe {
        TIMER_INFO[0].store(info.add(8).cast::<i32>().read() as i64 as u64, SeqCst);
        TIMER_INFO[1].store(info.add(16).cast::<i32>().read() as u64, SeqCst);
        TIMER_INFO[2].store(info.add(24).cast::<u64>().read(), SeqCst);
    }
    TIMER_SIGNALS.fetch_add(1, SeqCst);
}

global_asm!(".globl time_test_restorer", "time_test_restorer:", "mov eax, 15", "syscall", "ud2");

unsafe extern "C" {
    fn time_test_restorer();
}

fn handle(signal: u64, handler: u64, flags: u64) -> i64 {
    let act =
        SigAction { handler, flags: flags | SA_RESTORER, restorer: time_test_restorer as *const () as u64, mask: 0 };
    sys(SYS_RT_SIGACTION, &[signal, &raw const act as u64, 0, 8])
}

/// Work in user mode for a while, without system calls.
fn spin() {
    let mut x = 1u64;
    for i in 0..20_000u64 {
        x = core::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(i));
    }
    core::hint::black_box(x);
}

/// Spin (mostly in user mode) until `cond` holds or `limit` ns pass;
/// whether it held.
fn wait_for(limit: u64, cond: impl Fn() -> bool) -> bool {
    let start = mono();
    while !cond() {
        if mono() - start > limit {
            return false;
        }
        spin();
    }
    true
}

fn timer_tests() {
    // ITIMER_REAL: SIGALRM interrupts a long nanosleep, which reports the
    // time left.
    let r = handle(SIGALRM, on_alarm as *const () as u64, 0);
    check(b"sigaction_sigalrm", r == 0, r);
    let it: [i64; 4] = [0, 0, 0, 50_000]; // interval 0, value 50 ms
    let t0 = mono();
    let r = sys(SYS_SETITIMER, &[ITIMER_REAL, &raw const it as u64, 0]);
    check(b"setitimer_real", r == 0, r);
    let long = Timespec::from_ns(5 * SEC);
    let mut rem = Timespec::default();
    let r = sys(SYS_NANOSLEEP, &[&raw const long as u64, &raw mut rem as u64]);
    let waited = mono() - t0;
    check(b"itimer_real_delivers_sigalrm", ALARMS.load(SeqCst) == 1, ALARMS.load(SeqCst) as i64);
    check(b"sigalrm_interrupts_nanosleep_with_eintr", r == -EINTR && waited >= 50 * MS, r);
    check(b"nanosleep_reports_the_time_left", rem.ns() > 0 && rem.ns() < 5 * SEC, rem.ns() as i64);
    let mut cur = [-1i64; 4];
    sys(SYS_GETITIMER, &[ITIMER_REAL, &raw mut cur as u64]);
    check(b"a_fired_one_shot_itimer_is_disarmed", cur == [0; 4], cur[3]);

    // alarm: the time left of the previous one.
    let first = sys(SYS_ALARM, &[30]);
    let left = sys(SYS_ALARM, &[0]);
    check(b"alarm_reports_the_previous_alarm", first == 0 && (29..=30).contains(&left), left);

    // A periodic POSIX timer with SIGEV_SIGNAL: SIGUSR1 with SI_TIMER,
    // the timer id and the sigev_value.
    let r = handle(SIGUSR1, on_timer as *const () as u64, SA_SIGINFO);
    check(b"sigaction_sigusr1", r == 0, r);
    let mut ev = [0u8; 64];
    ev[0..8].copy_from_slice(&0x7777u64.to_le_bytes());
    ev[8..12].copy_from_slice(&(SIGUSR1 as i32).to_le_bytes());
    ev[12..16].copy_from_slice(&SIGEV_SIGNAL.to_le_bytes());
    let mut id: i32 = -1;
    let r = sys(SYS_TIMER_CREATE, &[CLOCK_MONOTONIC, ev.as_ptr() as u64, &raw mut id as u64]);
    check(b"timer_create", r == 0 && id >= 0, r);
    let spec = [Timespec::from_ns(10 * MS), Timespec::from_ns(20 * MS)]; // interval, value
    let r = sys(SYS_TIMER_SETTIME, &[id as u64, 0, spec.as_ptr() as u64, 0]);
    check(b"timer_settime", r == 0, r);
    let mut cur = [Timespec::default(); 2];
    let r = sys(SYS_TIMER_GETTIME, &[id as u64, cur.as_mut_ptr() as u64]);
    check(
        b"timer_gettime",
        r == 0 && cur[1].ns() > 0 && cur[1].ns() <= 20 * MS && cur[0].ns() == 10 * MS,
        cur[1].ns() as i64,
    );
    let fired = wait_for(10 * SEC, || TIMER_SIGNALS.load(SeqCst) >= 3);
    check(b"posix_timer_fires_periodically", fired, TIMER_SIGNALS.load(SeqCst) as i64);
    let code = TIMER_INFO[0].load(SeqCst) as i64 as i32;
    check(
        b"posix_timer_siginfo",
        code == SI_TIMER && TIMER_INFO[1].load(SeqCst) == id as u64 && TIMER_INFO[2].load(SeqCst) == 0x7777,
        code as i64,
    );
    let r = sys(SYS_TIMER_DELETE, &[id as u64]);
    check(b"timer_delete", r == 0, r);
    let count = TIMER_SIGNALS.load(SeqCst);
    let nap = Timespec::from_ns(50 * MS);
    sys(SYS_NANOSLEEP, &[&raw const nap as u64, 0]);
    check(b"a_deleted_timer_stops", TIMER_SIGNALS.load(SeqCst) <= count + 1, TIMER_SIGNALS.load(SeqCst) as i64);
    let r = sys(SYS_TIMER_DELETE, &[id as u64]);
    check(b"timer_delete_twice_is_einval", r == -22, r);

    // ITIMER_VIRTUAL counts user CPU time: a busy loop gets SIGVTALRM.
    let r = handle(SIGVTALRM, on_vtalarm as *const () as u64, 0);
    check(b"sigaction_sigvtalrm", r == 0, r);
    let it: [i64; 4] = [0, 0, 0, 20_000];
    let r = sys(SYS_SETITIMER, &[ITIMER_VIRTUAL, &raw const it as u64, 0]);
    let got = r == 0 && wait_for(20 * SEC, || VTALARMS.load(SeqCst) >= 1);
    check(b"itimer_virtual_counts_cpu_time", got, r);
}

// ---- CPU time ------------------------------------------------------------------------

fn cpu_tests() {
    // A busy thread's CPU-time clocks advance.
    let (thread0, process0, wall0) = (clock(CLOCK_THREAD_CPUTIME_ID), clock(CLOCK_PROCESS_CPUTIME_ID), mono());
    while mono() - wall0 < 100 * MS {
        spin();
    }
    let (thread1, process1, wall1) = (clock(CLOCK_THREAD_CPUTIME_ID), clock(CLOCK_PROCESS_CPUTIME_ID), mono());
    let (ran, wall) = (thread1 - thread0, wall1 - wall0);
    check(b"thread_cpu_clock_advances_while_busy", ran >= wall / 10 && ran <= wall + 10 * MS, ran as i64);
    check(b"process_cpu_clock_advances_while_busy", process1 - process0 >= ran / 2, (process1 - process0) as i64);
    // But not while it sleeps.
    let t0 = clock(CLOCK_THREAD_CPUTIME_ID);
    let nap = Timespec::from_ns(50 * MS);
    sys(SYS_NANOSLEEP, &[&raw const nap as u64, 0]);
    let slept = clock(CLOCK_THREAD_CPUTIME_ID) - t0;
    check(b"thread_cpu_clock_stands_still_while_asleep", slept < 25 * MS, slept as i64);
    // getrusage and times see the same time, mostly user time.
    let mut ru = [0i64; 18];
    let r = sys(SYS_GETRUSAGE, &[0, ru.as_mut_ptr() as u64]);
    let (user, system) = ((ru[0] * 1_000_000 + ru[1]) as u64 * 1000, (ru[2] * 1_000_000 + ru[3]) as u64 * 1000);
    check(
        b"getrusage_self",
        r == 0 && user + system > 0 && user + system <= clock(CLOCK_PROCESS_CPUTIME_ID),
        (user / 1000) as i64,
    );
    // Over the whole run the sleeps, timers and exec count as system time,
    // so compare only what a busy loop with few system calls adds.
    let rusage = || {
        let mut ru = [0i64; 18];
        sys(SYS_GETRUSAGE, &[0, ru.as_mut_ptr() as u64]);
        ((ru[0] * 1_000_000 + ru[1]) as u64, (ru[2] * 1_000_000 + ru[3]) as u64)
    };
    let ((user0, system0), wall0) = (rusage(), mono());
    while mono() - wall0 < 200 * MS {
        for _ in 0..50 {
            spin();
        }
    }
    let (user1, system1) = rusage();
    let (busy_user, busy_system) = (user1 - user0, system1 - system0);
    check(b"busy_loops_are_user_time", busy_user >= busy_system, busy_system as i64);
    let mut tms = [0i64; 4];
    let ticks = sys(SYS_TIMES, &[tms.as_mut_ptr() as u64]);
    check(b"times", ticks > 0 && tms[0] + tms[1] > 0, ticks);
}

#[unsafe(no_mangle)]
extern "C" fn main(sp: *const u64) -> ! {
    vdso_tests(sp);
    sleep_tests();
    timer_tests();
    cpu_tests();
    let failed = FAILED.load(SeqCst);
    write(b"time-test: done, ");
    write_num(failed as i64);
    write(b" failed\n");
    loop {
        sys(SYS_EXIT_GROUP, &[failed]);
    }
}

// The entry point: `rsp` points at `argc` (psABI §3.4.1); `main` gets it
// and a normal call frame.
global_asm!(".globl _start", "_start:", "xor ebp, ebp", "mov rdi, rsp", "and rsp, -16", "call main", "ud2");

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    write(b"time-test: FAIL panic\n");
    loop {
        sys(SYS_EXIT_GROUP, &[99]);
    }
}
