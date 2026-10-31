//! ADR 0092's slice of the runtime, checked from ring 3: the transcendental
//! math (values, `errno`, the program's `_matherr`), positional `printf`
//! (`__stdio_common_vsprintf_p`, `msvcrt.dll`'s `_sprintf_p_l`/`_vsprintf_p`/
//! `_scprintf_p_l`), and `setjmp`/`longjmp` both plain and unwinding
//! through a frame whose language handler must see the `STATUS_LONGJUMP`
//! unwind, from the UCRT and from `msvcrt.dll`.
//!
//! Its steps are numbered from 300 (own counter), so failures read
//! `crttest: step 3NN failed` and the exit code is `0x80 + 3NN`.

use super::{_errno, ERANGE, GetProcAddress, INVALID, LoadLibraryW, OPTS, cs, fail, ws};
use core::f64::consts::{E, FRAC_PI_2, LN_10, PI, SQRT_2};
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

const EDOM: i32 = 33;
/// `STATUS_LONGJUMP` (`ntstatus.h`), `EXCEPTION_UNWINDING` (`winnt.h`).
const STATUS_LONGJUMP: u32 = 0x8000_0026;
const EXCEPTION_UNWINDING: u32 = 2;

static STEP: AtomicU32 = AtomicU32::new(299);
/// What the language handler of `crt_sj_middle` saw: calls, last code and
/// flags, the record's first parameter.
static HANDLED: AtomicU32 = AtomicU32::new(0);
static HANDLED_CODE: AtomicU32 = AtomicU32::new(0);
static HANDLED_FLAGS: AtomicU32 = AtomicU32::new(0);
static HANDLED_INFO: AtomicU64 = AtomicU64::new(0);
/// The value `crt_sj_middle` passes to `longjmp`.
static CRT_SJ_VALUE: AtomicI32 = AtomicI32::new(7);

fn check(ok: bool) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        fail(step);
    }
}

#[link(name = "api-ms-win-crt-math-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn __setusermatherr(f: usize);
    fn sin(x: f64) -> f64;
    fn cos(x: f64) -> f64;
    fn tan(x: f64) -> f64;
    fn asin(x: f64) -> f64;
    fn acos(x: f64) -> f64;
    fn atan2(y: f64, x: f64) -> f64;
    fn sinh(x: f64) -> f64;
    fn tanh(x: f64) -> f64;
    fn exp(x: f64) -> f64;
    fn exp2(x: f64) -> f64;
    fn expm1(x: f64) -> f64;
    fn log(x: f64) -> f64;
    fn log2(x: f64) -> f64;
    fn log10(x: f64) -> f64;
    fn log1p(x: f64) -> f64;
    fn pow(x: f64, y: f64) -> f64;
    fn cbrt(x: f64) -> f64;
    fn hypot(x: f64, y: f64) -> f64;
    fn remainder(x: f64, y: f64) -> f64;
    fn sinf(x: f32) -> f32;
    fn expf(x: f32) -> f32;
    fn powf(x: f32, y: f32) -> f32;
}

#[link(name = "api-ms-win-crt-runtime-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn setjmp(buf: *mut u8) -> i32;
    fn longjmp(buf: *mut u8, value: i32) -> !;
}

#[link(name = "api-ms-win-crt-stdio-l1-1-0", kind = "raw-dylib")]
unsafe extern "C" {
    fn __stdio_common_vsprintf_p(o: u64, b: *mut u8, n: usize, fmt: *const u8, l: *mut u8, va: *const u64) -> i32;
}

/// `math.h`'s `struct _exception`.
#[repr(C)]
struct Exception {
    kind: i32,
    name: *const u8,
    arg1: f64,
    arg2: f64,
    retval: f64,
}

/// The program's `_matherr`: a `_DOMAIN` error of `log` returns the log of
/// the absolute value (Learn's example), anything else the default.
extern "C" fn matherr(e: *mut Exception) -> i32 {
    // SAFETY: the runtime passes a valid _exception.
    let e = unsafe { &mut *e };
    if e.kind == 1 && cs(e.name) == b"log" {
        e.retval = 42.0 + e.arg1;
        return 1;
    }
    0
}

/// The language handler of `crt_sj_middle`: records the unwind it sees.
extern "C" fn sj_handler(record: *const u8, _frame: u64, _ctx: *mut u8, _dc: *mut u8) -> i32 {
    // SAFETY: an EXCEPTION_RECORD (`winnt.h`: code at 0, flags at 4,
    // ExceptionInformation at 0x20).
    unsafe {
        HANDLED_CODE.store((record as *const u32).read(), Ordering::Relaxed);
        HANDLED_FLAGS.store((record.add(4) as *const u32).read(), Ordering::Relaxed);
        HANDLED_INFO.store((record.add(0x20) as *const u64).read(), Ordering::Relaxed);
    }
    HANDLED.fetch_add(1, Ordering::Relaxed);
    1 // ExceptionContinueSearch
}

unsafe extern "C" {
    /// `crt_sj_outer(buf, setjmp, longjmp, with_frame)`: calls `setjmp(buf,
    /// with_frame ? its frame : 0)` with R12 = 0x1234, then (R12 = 0x5678)
    /// `crt_sj_middle(buf, longjmp)`, which calls `longjmp(buf,
    /// CRT_SJ_VALUE)`. Returns `setjmp`'s second value, plus 1000 if R12
    /// was not restored to 0x1234; −1 if `longjmp` returned.
    fn crt_sj_outer(buf: *mut u8, setjmp: usize, longjmp: usize, with_frame: usize) -> i32;
}

// Assembly, so that Rust never sees a function that returns twice. The
// outer function has no frame pointer: its establisher frame is RSP after
// the prologue, which it passes to setjmp as MSVC's code does.
core::arch::global_asm!(
    ".text",
    ".globl crt_sj_outer",
    ".p2align 4",
    ".seh_proc crt_sj_outer",
    "crt_sj_outer:",
    "push rbx",
    ".seh_pushreg rbx",
    "push rsi",
    ".seh_pushreg rsi",
    "push r12",
    ".seh_pushreg r12",
    "push r13",
    ".seh_pushreg r13",
    "sub rsp, 40",
    ".seh_stackalloc 40",
    ".seh_endprologue",
    "mov rbx, rcx",
    "mov rsi, r8",
    "mov r13, rdx",
    "mov r12, 0x1234",
    "xor edx, edx",
    "test r9, r9",
    "jz .Lcrt_sj_call",
    "mov rdx, rsp",
    ".Lcrt_sj_call:",
    "mov rcx, rbx",
    "call r13",
    "test eax, eax",
    "jnz .Lcrt_sj_again",
    "mov r12, 0x5678",
    "mov rcx, rbx",
    "mov rdx, rsi",
    "call crt_sj_middle",
    "mov eax, -1",
    "jmp .Lcrt_sj_out",
    ".Lcrt_sj_again:",
    "cmp r12, 0x1234",
    "je .Lcrt_sj_out",
    "add eax, 1000",
    ".Lcrt_sj_out:",
    "add rsp, 40",
    "pop r13",
    "pop r12",
    "pop rsi",
    "pop rbx",
    "ret",
    ".seh_endproc",
    "",
    ".p2align 4",
    ".seh_proc crt_sj_middle",
    "crt_sj_middle:",
    "sub rsp, 40",
    ".seh_stackalloc 40",
    ".seh_endprologue",
    "mov rax, rdx",
    "mov edx, dword ptr [rip + {value}]",
    "call rax",
    "int3",
    "add rsp, 40",
    "ret",
    ".seh_handler {handler}, @unwind",
    ".seh_endproc",
    value = sym CRT_SJ_VALUE,
    handler = sym sj_handler,
);

/// One `setjmp`/`longjmp` round through the assembly: the value `setjmp`
/// returned the second time, and how many unwind calls the handler saw.
fn jump(set: usize, long: usize, with_frame: bool, value: i32) -> (i32, u32) {
    let mut buf = [0u8; 256 + 16];
    // 16-byte aligned, as `jmp_buf`'s SETJMP_FLOAT128 elements are.
    let at = (buf.as_mut_ptr() as usize + 15) & !15;
    let before = HANDLED.load(Ordering::Relaxed);
    CRT_SJ_VALUE.store(value, Ordering::Relaxed);
    // SAFETY: the assembly keeps every register the x64 convention asks it
    // to; `set` and `long` are the runtime's setjmp and longjmp.
    let r = unsafe { crt_sj_outer(at as *mut u8, set, long, usize::from(with_frame)) };
    let seen = HANDLED.load(Ordering::Relaxed) - before;
    if with_frame && seen > 0 {
        let ok = HANDLED_CODE.load(Ordering::Relaxed) == STATUS_LONGJUMP
            && HANDLED_FLAGS.load(Ordering::Relaxed) & EXCEPTION_UNWINDING != 0
            && HANDLED_INFO.load(Ordering::Relaxed) == at as u64;
        if !ok {
            return (r, 99);
        }
    }
    (r, seen)
}

/// Arguments the compiler cannot see, so it does not fold the calls with
/// its own idea of the functions (LLVM evaluates `sinh(710.0)` and friends
/// at compile time with the build machine's library).
fn b(x: f64) -> f64 {
    core::hint::black_box(x)
}

fn bf(x: f32) -> f32 {
    core::hint::black_box(x)
}

pub fn run() {
    // SAFETY: the runtime's documented functions with valid arguments.
    unsafe {
        // ----- values: correctly rounded, as poly-libm's tests show -----
        check(sin(b(1.0)) == 0.8414709848078965 && cos(b(PI)) == -1.0 && tan(b(PI / 4.0)) == 0.9999999999999999); // 300
        check(asin(b(1.0)) == FRAC_PI_2 && acos(b(-1.0)) == PI && atan2(b(1.0), b(-1.0)) == 2.356194490192345);
        check(exp(b(1.0)) == E && log(b(10.0)) == LN_10 && log10(b(1e22)) == 22.0 && log2(b(8.0)) == 3.0);
        check(pow(b(2.0), b(0.5)) == SQRT_2 && pow(b(10.0), b(22.0)) == 1e22 && pow(b(-2.0), b(3.0)) == -8.0);
        check(exp2(b(10.0)) == 1024.0 && expm1(b(1e-300)) == 1e-300 && log1p(b(1e-300)) == 1e-300);
        check(sinh(b(710.0)) == 1.1169973830808555e308 && tanh(b(-30.0)) == -1.0); // 305
        check(cbrt(b(27.0)) == 3.0 && hypot(b(3.0), b(4.0)) == 5.0 && remainder(b(7.0), b(2.0)) == -1.0);
        check(sin(b(1e22)) == -0.8522008497671888 && sin(b(-0.0)).to_bits() == (-0.0f64).to_bits());
        check(
            sinf(bf(1.0)).to_bits() == 0x3F57_6AA4
                && expf(bf(1.0)).to_bits() == 0x402D_F854
                && powf(bf(2.0), bf(10.0)) == 1024.0,
        );

        // ----- errno and _matherr -----
        *_errno() = 0;
        check(log(b(0.0)) == f64::NEG_INFINITY && *_errno() == ERANGE);
        *_errno() = 0;
        check(log(b(-1.0)).is_nan() && *_errno() == EDOM); // 310
        *_errno() = 0;
        check(exp(b(1000.0)) == f64::INFINITY && *_errno() == ERANGE);
        *_errno() = 0;
        check(asin(b(2.0)).is_nan() && *_errno() == EDOM);
        *_errno() = 0;
        check(pow(b(-2.0), b(0.5)).is_nan() && *_errno() == EDOM && sin(b(2.0)) > 0.9 && *_errno() == EDOM);
        __setusermatherr(matherr as extern "C" fn(*mut Exception) -> i32 as usize);
        *_errno() = 0;
        check(log(b(-1.0)) == 41.0 && *_errno() == 0);
        check(log(b(0.0)) == f64::NEG_INFINITY && *_errno() == ERANGE); // 315: _SING is not handled
        __setusermatherr(0);
        *_errno() = 0;
        check(log(b(-1.0)).is_nan() && *_errno() == EDOM);

        // ----- positional printf -----
        let mut buf = [0xAAu8; 64];
        let args = [5u64, c"x".as_ptr() as u64];
        let n = __stdio_common_vsprintf_p(
            OPTS,
            buf.as_mut_ptr(),
            64,
            c"%2$s %1$d %1$03d".as_ptr() as *const u8,
            core::ptr::null_mut(),
            args.as_ptr(),
        );
        check(n == 7 && &buf[..8] == b"x 5 005\0");
        let before = INVALID.load(Ordering::Relaxed);
        let n = __stdio_common_vsprintf_p(
            OPTS,
            buf.as_mut_ptr(),
            64,
            c"%1$d %d".as_ptr() as *const u8,
            core::ptr::null_mut(),
            args.as_ptr(),
        );
        check(n == -1 && INVALID.load(Ordering::Relaxed) == before + 1);

        // ----- setjmp / longjmp (UCRT) -----
        let (set, long) = (
            setjmp as unsafe extern "C" fn(*mut u8) -> i32 as usize,
            longjmp as unsafe extern "C" fn(*mut u8, i32) -> ! as usize,
        );
        check(jump(set, long, false, 7) == (7, 0)); // 320
        check(jump(set, long, false, 0) == (1, 0));
        check(jump(set, long, true, 9) == (9, 1));

        // ----- msvcrt.dll -----
        let m = LoadLibraryW(ws(b"msvcrt.dll\0").as_ptr());
        check(!m.is_null());
        let get = |n: &core::ffi::CStr| GetProcAddress(m, n.as_ptr() as *const u8) as usize;
        let (mset, mlong) = (get(c"_setjmp"), get(c"longjmp"));
        check(mset != 0 && mlong != 0 && get(c"_setjmpex") == mset);
        check(jump(mset, mlong, true, 3) == (3, 1)); // 325
        check(jump(mset, mlong, false, 4) == (4, 0));
        type SprintfP = unsafe extern "C" fn(*mut u8, usize, *const u8, *mut u8, ...) -> i32;
        type VsprintfP = unsafe extern "C" fn(*mut u8, usize, *const u8, *const u64) -> i32;
        type ScprintfP = unsafe extern "C" fn(*const u8, *mut u8, ...) -> i32;
        let sprintf_p: SprintfP = core::mem::transmute(get(c"_sprintf_p_l"));
        let vsprintf_p: VsprintfP = core::mem::transmute(get(c"_vsprintf_p"));
        let scprintf_p: ScprintfP = core::mem::transmute(get(c"_scprintf_p_l"));
        let mut out = [0u8; 32];
        let r = sprintf_p(
            out.as_mut_ptr(),
            32,
            c"%3$s-%1$.1f-%2$x".as_ptr() as *const u8,
            core::ptr::null_mut(),
            2.2f64,
            255,
            c"z".as_ptr(),
        );
        check(r == 8 && &out[..9] == b"z-2.2-ff\0");
        let r = vsprintf_p(out.as_mut_ptr(), 32, c"%2$s%1$d".as_ptr() as *const u8, args.as_ptr());
        check(r == 2 && &out[..3] == b"x5\0");
        let r = scprintf_p(c"%2$d%1$d".as_ptr() as *const u8, core::ptr::null_mut(), 12, 345);
        check(r == 5);
    }
}
