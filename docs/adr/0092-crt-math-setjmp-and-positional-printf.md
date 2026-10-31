# 0092: The C runtime's transcendental math in a double-double `poly-libm`, `setjmp`/`longjmp` that unwind, positional `printf`

**Status:** Accepted. A slice of the Phase 5 item "C runtime: `ucrtbase` +
`api-ms-win-crt-*` and `msvcrt`" on top of [ADR 0084](0084-our-c-runtime-ucrtbase-and-msvcrt.md)
(the runtime's structure, the export checks); uses [ADR 0068](0068-nt-structured-exception-handling-and-x64-unwinding.md)'s
unwinding through our `ntdll`'s `RtlUnwindEx`.

## Context

ADR 0084's first slice left out three things this slice delivers:

- the transcendental functions of `math.h` (`sin` … `pow`, the C99
  additions, their `float` and `long double` forms) with `errno` and the
  program's `_matherr`;
- `setjmp`/`longjmp`, which on x64 Windows "uses the same stack-unwinding
  semantics as exception-handling code" (Learn, `longjmp`);
- positional `printf` (`%2$s`, `*1$`; Learn, "printf_p Positional
  Parameters"): the UCRT's `__stdio_common_v*printf_p`, `msvcrt.dll`'s
  `_fprintf_p`, `_sprintf_p_l`, `_vsprintf_p` and friends.

Sources (clean room, CLEAN_ROOM.md): Learn's CRT reference (each
function's page, its "Input / SEH exception / `_matherr` exception" table,
`_matherr`, `setjmp`, `longjmp`, `_sprintf_p`, "printf_p Positional
Parameters", "Data type ranges"), the C standard (C11 7.12, 7.13, annex F),
IEEE 754-2019, the public `setjmp.h` (`_JUMP_BUFFER`), `math.h`
(`struct _exception`, `_DOMAIN` …), `winnt.h` (`EXCEPTION_RECORD`,
`CONTEXT`, `NT_TIB`) and `ntstatus.h` (`STATUS_LONGJUMP`); for the
numerics the textbook methods: Knuth's two-sum, Dekker's product and
fast two-sum, Payne and Hanek's radian reduction, Taylor series with
their remainder bounds. No libm or C runtime source (Wine, ReactOS, musl,
glibc, fdlibm, newlib, the UCRT) was read, and nothing was run on Windows.

## Decision

### `crates/poly-libm`: every function from first principles, rounded once

A `no_std` host-tested crate, no dependencies. Each function reduces its
argument exactly or in double-double (about 106 bits, `dd.rs`), sums a
Taylor series whose first omitted term is below 2^-90 (the leading terms in
double-double, the small tail in `f64`), and rounds the double-double
result once at the end (`dd::round_scaled`, which rounds once also into
the subnormal range):

| Functions | Method |
|---|---|
| `sin` `cos` `tan` | Payne–Hanek for every `|x| > π/4`: 192 bits of 2/π from the right place (1280 bits stored) times the 53-bit significand; `n ≤ 12`/`13` Taylor kernels; `tan` a double-double quotient |
| `exp` `exp2` `expm1` | `x = k ln 2 + r`, `|r| ≤ ln2/2`, Taylor to `r^19`; `expm1` without the constant term near 0 |
| `sinh` `cosh` `tanh` | from the exp kernels in double-double (`(E + E/(E+1))/2`, `E/(E+2)` with `E = expm1`), no cancellation |
| `log` `log2` `log10` `log1p` | `x = 2^k m`, `ln m = 2 atanh((m−1)/(m+1))`; `log2` adds `k` exactly; `log1p` takes `1 + x` exactly as a pair |
| `pow` | `exp(y ln|x|)` with both in double-double; C11 F.10.4.4's special cases |
| `atan` `atan2` `asin` `acos` | one kernel: two angle halvings then Taylor; `asin`/`acos` through `√((1−x)(1+x))` |
| `sqrt` `cbrt` `hypot` | integer square root (correctly rounded, no hardware needed); Newton plus one exact-residual correction; scaled sum of exact squares |
| `remainder` | exact, on the integer significands, quotient rounded to even |

**Accuracy.** The documented target is at most one ulp for every function
and argument. The design bound is 0.5 + 2^-20 ulp (each module states its
own), i.e. nearly always the correctly rounded result. The tests measure
it: 4,244 exact values (`tests/reference.txt`, made by
`tests/gen_reference.py` with Python's `decimal` at 70+ digits, π by
Machin's formula to 2500 bits for the reduction modulo 2π) are all
**correctly rounded** (worst 0.49993 ulp, `pow`); identities over 200,000
random arguments (sin² + cos², `cosh² − sinh²`, `exp(log x)`, inverses);
`sqrt` bit-identical to the hardware on 20,000 random doubles; exact
values (`log2(2^k)`, `log10(10^n)`, `exp2(k)`, cubes, `pow(10, 22)`); and
the hard reduction case `6381956970095103 × 2^797`. The `float` forms
compute in double and round again (within 0.5 + 2^-29 ulp of `float`);
`long double` is `double` (Learn), so the `l` forms are the double ones.

Performance is a secondary goal (double-double costs a few hundred
flops per call); the methods leave room for table-driven kernels later
without changing results.

### Errors: `poly_libm::fault`, `_matherr`, `errno`

`fault(function, x, y, result)` decides the `_exception` type a call
reports. Learn's tables are followed where they exist (`exp`, `log`,
`log10`, `pow`, `sin`/`cos`/`tan` and siblings): a NaN argument is
`_DOMAIN` (when the result is a NaN), ±infinity is `_DOMAIN` for `exp` and
the circular functions, ±0 is `_SING` for the logarithms, `exp`'s
`_OVERFLOW` and `_UNDERFLOW` where the result leaves the normal range. The
C99 additions follow C11 7.12.1 (no error for a NaN). The runtime
(`personalities/nt/win32/crt/src/mathfn.rs`) then calls the program's
handler installed with `__setusermatherr` with a `math.h`
`struct _exception` (`retval` is what the function returns, the handler
may change it; a nonzero return leaves `errno` alone, Learn), else sets
`errno`: `EDOM` for `_DOMAIN`, `ERANGE` for `_SING`, `_OVERFLOW` and
`_UNDERFLOW`.

### `setjmp`/`longjmp`

Assembly in the shared runtime (`jmp.rs`), exported by each DLL's
`build.rs` under the names its coverage list has (`msvcrt`: `setjmp`,
`_setjmp`, `_setjmpex`, `longjmp`; `ucrtbase`: `setjmp`,
`__intrinsic_setjmp`, `__intrinsic_setjmpex`, `longjmp`). The `jmp_buf`
is `setjmp.h`'s x64 `_JUMP_BUFFER` (256 bytes). `setjmp` stores `RDX`, the
frame MSVC's code passes, as `Frame`. `longjmp` with a nonzero `Frame`
that lies on this thread's stack (`NT_TIB.StackBase`) above it unwinds
with `RtlUnwindEx` (imported from `kernel32`, which forwards to `ntdll`)
and a `STATUS_LONGJUMP` record whose first parameter is the `jmp_buf`, so
termination handlers and C++ destructors run; the unwind resumes at a
landing stub in the target frame that restores the registers saved in
the `jmp_buf`. Otherwise it restores them directly. `value` 0 returns 1.
The value reaches the stub through a TLS slot of its own: the unwind needs
nothing from `ntdll` beyond the documented `RtlUnwindEx`, and nothing of
the C++ exception work.

### Positional `printf`

One more option bit for the shared engine, `poly_crt::printf::POSITIONAL`
(bit 63, Poly's own, beside the UCRT's `_CRT_INTERNAL_PRINTF_*`). With it,
`printf::positional` scans the format first: no numbered conversion means
the ordinary rules; otherwise every conversion and every `*` must be
numbered, each number from 1 to the highest used, at most `_ARGMAX` (100),
and a number may repeat (Learn). Since every x64 argument is one 8-byte
slot, the engine reads that many slots up front and picks them by number.
A violation is an invalid format (the invalid parameter handler, `EINVAL`,
−1). `_sprintf_p` is "`sprintf_s` with positional parameters" (Learn), so
`__stdio_common_vsprintf_p` uses the secure buffer rule, except the
counting form (no buffer, size 0), which follows the option bits.

## To be confirmed by the spec agent against real Windows

1. `_matherr`/`errno` for a NaN argument of the classic functions
   (Poly: `_DOMAIN`, `EDOM`, from Learn's tables), for `exp(±inf)`
   (Poly: `_DOMAIN` though the result is exact), for `log(0)` (Poly:
   `ERANGE`), for underflow (Poly: `ERANGE` when the result is subnormal
   or zero), and which functions call `_matherr` at all (Poly: all of
   this slice, including the `float` forms with names like `"sinf"`).
2. Results: Poly's are correctly rounded (bar ties within 2^-20 ulp);
   Windows' may differ in the last bit. `pow` of a negative base and an
   integral exponent of 2^64 or more (Learn: "pow doesn't recognize
   integral floating-point values greater than 2^64"; Poly follows C,
   such exponents are even).
3. `longjmp`'s registers after an unwinding jump (Poly: the `jmp_buf`'s),
   and a `Frame` that is not on the stack (Poly: a direct restore).
4. Positional formats: whether conversions sharing a number must agree in
   type, and the buffer rule of `__stdio_common_vsprintf_p` when the output
   does not fit (Poly: as `sprintf_s`).

## Consequences

- New host crate `crates/poly-libm` (21 tests including the reference
  set); `poly-crt`'s printf gains `POSITIONAL`, `ARGMAX` and `positional`
  (one more test with Learn's examples and the error cases).
- The ring-3 test (`crttest.exe`, its new `mathtest` module, steps 300 on)
  checks values, `errno` and a `_matherr` handler, positional `printf`
  through the UCRT and `msvcrt.dll`, and `setjmp`/`longjmp` plain and
  unwinding through a frame whose language handler must see the
  `STATUS_LONGJUMP` unwind, from both DLLs.
- Shared files changed additively: `poly_crt::exports::MSVCRT_VARARGS`
  (eight positional variadic functions), both DLLs' `build.rs` (the
  `setjmp` aliases), one `mod` line in each DLL and in `crttest`, the
  `MATHERR` static of `misc.rs` made crate-visible, the host workspace
  members.
- The coverage entries of this slice (115 in `msvcrt.dll` and
  `ucrtbase.dll`) are claimed by `impl-crt-math`.
- **Left** of the C runtime item: C++ exceptions, locales and multibyte
  code pages, file/process functions, console I/O (other slices).
