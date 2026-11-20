//! The format engine of the `printf` family, as the C standard (C11
//! 7.21.6.1) and the macOS `printf(3)` manual page define it, with the
//! arguments of the System V x86-64 calling convention (a `va_list` with a
//! register save area: integers and pointers in one sequence, `double`s in
//! another, `long double`s in memory).
//!
//! - flags `- + space # 0` (and `'`, grouping, which the C locale ignores);
//!   width and precision as digits or `*`;
//! - sizes `hh h l ll j z t L`, and `q` (as `ll`, `printf(3)`);
//! - conversions `d i o u x X c s p n % f F e E g G a A`, and `C S D O U`
//!   (as `lc ls ld lo lu`, `printf(3)`);
//! - `%p` is printed as by `%#lx` (`printf(3)`: "The void * pointer argument
//!   is printed in hexadecimal (as if by `%#x' or `%#lx')").
//!
//! Choices no public document settles, to be confirmed by the libc sheets
//! (spec-macos-5): a null `%p` prints `0x0`; a null `%s` prints `(null)`; a
//! NaN keeps its sign (`-nan`); `%a` normalizes subnormals to a leading `1`
//! and carries a rounding overflow into the exponent; a wide character
//! outside ASCII fails with `EILSEQ` (the C locale); an unknown conversion,
//! and positional arguments (`%1$d`), fail with `EINVAL`; `long double` is
//! converted to `double` first.

use core::fmt::Write as _;

/// Where the variadic arguments come from.
pub trait Args {
    /// The next integer or pointer argument (`INTEGER` class).
    fn int(&mut self) -> u64;
    /// The next `double` (`SSE` class).
    fn double(&mut self) -> f64;
    /// The next `long double` (`X87` class, always in memory), as a
    /// `double`.
    fn long_double(&mut self) -> f64;
}

/// Where the output goes.
pub trait Sink {
    fn put(&mut self, bytes: &[u8]);
}

/// Why formatting stopped (the `errno` the function sets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// An unknown conversion, or positional arguments: `EINVAL`.
    Invalid,
    /// A wide character with no form in the C locale: `EILSEQ`.
    Encoding,
}

/// The System V x86-64 `va_list` element (`__va_list_tag`, ABI 3.5.7).
#[repr(C)]
#[derive(Debug)]
pub struct VaListTag {
    pub gp_offset: u32,
    pub fp_offset: u32,
    pub overflow_arg_area: *mut u8,
    pub reg_save_area: *mut u8,
}

/// The end of the integer registers in the save area (6 × 8 bytes).
pub const GP_END: u32 = 48;
/// The end of the vector registers in the save area (48 + 8 × 16 bytes).
pub const FP_END: u32 = 176;

/// A caller's `va_list`, read as the ABI's `va_arg` algorithm does.
#[derive(Debug)]
pub struct VaList(pub *mut VaListTag);

impl VaList {
    /// The next 8 bytes from the overflow area.
    ///
    /// # Safety
    /// The area holds the argument.
    unsafe fn overflow(&mut self, size: usize, align: usize) -> *const u8 {
        // SAFETY: `VaList` is only made from a live va_list (see `Args`).
        let tag = unsafe { &mut *self.0 };
        let at = (tag.overflow_arg_area as usize).next_multiple_of(align) as *mut u8;
        // SAFETY: still inside the caller's argument area.
        tag.overflow_arg_area = unsafe { at.add(size.next_multiple_of(8)) };
        at
    }
}

impl Args for VaList {
    fn int(&mut self) -> u64 {
        // SAFETY: a `VaList` is made from the va_list of a call whose format
        // names as many arguments as are read (C11 7.16.1.1).
        let tag = unsafe { &mut *self.0 };
        if tag.gp_offset < GP_END {
            // SAFETY: inside the register save area.
            let v = unsafe { (tag.reg_save_area.add(tag.gp_offset as usize) as *const u64).read_unaligned() };
            tag.gp_offset += 8;
            return v;
        }
        // SAFETY: as above, in the overflow area.
        unsafe { (self.overflow(8, 8) as *const u64).read_unaligned() }
    }

    fn double(&mut self) -> f64 {
        // SAFETY: as for `int`.
        let tag = unsafe { &mut *self.0 };
        if tag.fp_offset < FP_END {
            // SAFETY: inside the register save area.
            let v = unsafe { (tag.reg_save_area.add(tag.fp_offset as usize) as *const f64).read_unaligned() };
            tag.fp_offset += 16;
            return v;
        }
        // SAFETY: as above, in the overflow area.
        unsafe { (self.overflow(8, 8) as *const f64).read_unaligned() }
    }

    fn long_double(&mut self) -> f64 {
        // SAFETY: as for `int`; a long double takes 16 bytes, 16-aligned.
        let p = unsafe { self.overflow(16, 16) };
        // SAFETY: the 10 bytes of the x87 value.
        let (m, se) = unsafe { ((p as *const u64).read_unaligned(), (p.add(8) as *const u16).read_unaligned()) };
        x87_to_f64(m, se)
    }
}

/// An x87 extended value (64-bit significand with its explicit integer bit,
/// sign and 15-bit exponent) as the nearest `double`.
pub fn x87_to_f64(mantissa: u64, sign_exp: u16) -> f64 {
    let neg = sign_exp & 0x8000 != 0;
    let e = i32::from(sign_exp & 0x7fff);
    let v = if e == 0x7fff {
        if mantissa << 1 == 0 { f64::INFINITY } else { f64::NAN }
    } else if mantissa == 0 {
        0.0
    } else {
        // mantissa × 2^(e - 16383 - 63), in two steps so neither overflows
        // too early.
        let m = mantissa as f64;
        let k = e - 16383 - 63;
        scale(m, k)
    };
    if neg { -v } else { v }
}

fn scale(mut v: f64, mut k: i32) -> f64 {
    while k > 0 {
        let s = k.min(1000);
        v *= f64::from_bits(((1023 + s) as u64) << 52);
        k -= s;
        if v.is_infinite() {
            return v;
        }
    }
    while k < 0 {
        let s = (-k).min(1000);
        v /= f64::from_bits(((1023 + s) as u64) << 52);
        k += s;
        if v == 0.0 {
            return v;
        }
    }
    v
}

/// Apple's arm64 `va_list` (Apple's "Writing ARM64 code for Apple
/// platforms": variadic arguments are passed on the stack, each in its own
/// 8-byte slot, and `va_list` is a plain pointer to the next one;
/// `long double` is `double`). Reading advances the pointer.
#[derive(Debug)]
pub struct StackSlots(pub *const u8);

impl StackSlots {
    fn next(&mut self) -> [u8; 8] {
        // SAFETY: a `StackSlots` is made from the va_list of a call whose
        // format names as many arguments as are read (C11 7.16.1.1); each
        // takes one 8-byte slot.
        let v = unsafe { (self.0 as *const [u8; 8]).read_unaligned() };
        // SAFETY: still inside the caller's argument area (as above).
        self.0 = unsafe { self.0.add(8) };
        v
    }
}

impl Args for StackSlots {
    fn int(&mut self) -> u64 {
        u64::from_le_bytes(self.next())
    }
    fn double(&mut self) -> f64 {
        f64::from_le_bytes(self.next())
    }
    fn long_double(&mut self) -> f64 {
        self.double()
    }
}

/// Arguments from slices: integers and doubles in their own order (tests,
/// and callers that built them).
#[derive(Debug)]
pub struct Slots<'a> {
    pub ints: &'a [u64],
    pub doubles: &'a [f64],
}

impl Args for Slots<'_> {
    fn int(&mut self) -> u64 {
        let (&v, rest) = self.ints.split_first().unwrap_or((&0, &[]));
        self.ints = rest;
        v
    }
    fn double(&mut self) -> f64 {
        let (&v, rest) = self.doubles.split_first().unwrap_or((&0.0, &[]));
        self.doubles = rest;
        v
    }
    fn long_double(&mut self) -> f64 {
        self.double()
    }
}

/// A sink that only counts.
#[derive(Debug, Default)]
pub struct Count(pub usize);

impl Sink for Count {
    fn put(&mut self, bytes: &[u8]) {
        self.0 += bytes.len();
    }
}

/// A sink into a buffer that keeps what fits (`snprintf`): the buffer's
/// bytes, how many were stored.
#[derive(Debug)]
pub struct Buffer<'a> {
    pub buf: &'a mut [u8],
    pub len: usize,
}

impl Sink for Buffer<'_> {
    fn put(&mut self, bytes: &[u8]) {
        let room = self.buf.len() - self.len;
        let n = room.min(bytes.len());
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Size {
    None,
    Hh,
    H,
    L,
    Ll,
    BigL,
    J,
    Z,
    T,
}

#[derive(Clone, Copy, Debug, Default)]
struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

/// The output and the count so far (for `%n` and the result).
struct Out<'a, S> {
    sink: &'a mut S,
    count: usize,
}

impl<S: Sink> Out<'_, S> {
    fn put(&mut self, b: &[u8]) {
        self.count += b.len();
        self.sink.put(b);
    }
    fn pad(&mut self, c: u8, n: usize) {
        let chunk = [c; 32];
        let mut left = n;
        while left > 0 {
            let k = left.min(32);
            self.put(&chunk[..k]);
            left -= k;
        }
    }
}

/// Format `fmt` (without its NUL) into `out`; the number of bytes the full
/// output has (C: what `printf` returns, before any check against
/// `INT_MAX`).
///
/// # Safety
/// The arguments are those `fmt` describes (C11 7.21.6.1p9): string and
/// `%n` arguments are valid pointers (a string may be null).
pub unsafe fn format<A: Args, S: Sink>(fmt: &[u8], args: &mut A, sink: &mut S) -> Result<usize, Error> {
    let mut out = Out { sink, count: 0 };
    let mut i = 0;
    while i < fmt.len() {
        let start = i;
        while i < fmt.len() && fmt[i] != b'%' {
            i += 1;
        }
        if i > start {
            out.put(&fmt[start..i]);
        }
        if i >= fmt.len() {
            break;
        }
        i += 1; // '%'
        let mut spec = Spec::default();
        // Positional arguments (`%1$d`) are not supported.
        let mut j = i;
        while j < fmt.len() && fmt[j].is_ascii_digit() {
            j += 1;
        }
        if j > i && fmt.get(j) == Some(&b'$') {
            return Err(Error::Invalid);
        }
        while let Some(&c) = fmt.get(i) {
            match c {
                b'-' => spec.left = true,
                b'+' => spec.plus = true,
                b' ' => spec.space = true,
                b'#' => spec.alt = true,
                b'0' => spec.zero = true,
                b'\'' => {}
                _ => break,
            }
            i += 1;
        }
        if fmt.get(i) == Some(&b'*') {
            i += 1;
            let w = args.int() as i32;
            if w < 0 {
                spec.left = true;
            }
            spec.width = w.unsigned_abs() as usize;
        } else {
            spec.width = number(fmt, &mut i);
        }
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            if fmt.get(i) == Some(&b'*') {
                i += 1;
                let p = args.int() as i32;
                spec.precision = (p >= 0).then_some(p as usize);
            } else {
                spec.precision = Some(number(fmt, &mut i));
            }
        }
        let mut size = Size::None;
        loop {
            let s = match (size, fmt.get(i)) {
                (Size::None, Some(b'h')) => Size::H,
                (Size::H, Some(b'h')) => Size::Hh,
                (Size::None, Some(b'l')) => Size::L,
                (Size::L, Some(b'l')) => Size::Ll,
                (Size::None, Some(b'q')) => Size::Ll,
                (Size::None, Some(b'L')) => Size::BigL,
                (Size::None, Some(b'j')) => Size::J,
                (Size::None, Some(b'z')) => Size::Z,
                (Size::None, Some(b't')) => Size::T,
                _ => break,
            };
            size = s;
            i += 1;
        }
        let Some(&conv) = fmt.get(i) else { return Err(Error::Invalid) };
        i += 1;
        // `printf(3)`'s deprecated capitals: `C S D O U` are `lc ls ld lo lu`.
        let (conv, size) = match conv {
            b'C' => (b'c', Size::L),
            b'S' => (b's', Size::L),
            b'D' => (b'd', Size::L),
            b'O' => (b'o', Size::L),
            b'U' => (b'u', Size::L),
            c => (c, size),
        };
        match conv {
            b'%' => out.put(b"%"),
            b'd' | b'i' => {
                let v = signed(args.int(), size);
                let sign: &[u8] = if v < 0 {
                    b"-"
                } else if spec.plus {
                    b"+"
                } else if spec.space {
                    b" "
                } else {
                    b""
                };
                integer(&mut out, spec, sign, b"", v.unsigned_abs(), 10, false);
            }
            b'u' => integer(&mut out, spec, b"", b"", unsigned(args.int(), size), 10, false),
            b'o' => {
                let v = unsigned(args.int(), size);
                integer(&mut out, spec, b"", b"", v, 8, false);
            }
            b'x' | b'X' => {
                let v = unsigned(args.int(), size);
                let upper = conv == b'X';
                let prefix: &[u8] = match (spec.alt && v != 0, upper) {
                    (false, _) => b"",
                    (true, false) => b"0x",
                    (true, true) => b"0X",
                };
                integer(&mut out, spec, b"", prefix, v, 16, upper);
            }
            b'p' => {
                let v = args.int();
                integer(&mut out, spec, b"", b"0x", v, 16, false);
            }
            b'c' => {
                let v = args.int();
                let mut b = [0u8; 1];
                if size == Size::L {
                    b[0] = narrow(v as u32)?;
                } else {
                    b[0] = v as u8;
                }
                field(&mut out, spec, &[], &[], &b, 0);
            }
            b's' => {
                let p = args.int() as *const u8;
                if size == Size::L {
                    // SAFETY: the caller's contract: a wide string or null.
                    unsafe { wide_string(&mut out, spec, p as *const u32)? };
                } else {
                    let s: &[u8] = if p.is_null() {
                        b"(null)"
                    } else {
                        // SAFETY: the caller's contract: a string, read no
                        // further than the precision.
                        unsafe { bytes_until_nul(p, spec.precision) }
                    };
                    let s = &s[..spec.precision.map_or(s.len(), |n| n.min(s.len()))];
                    field(&mut out, spec, &[], &[], s, 0);
                }
            }
            b'n' => {
                let p = args.int() as *mut u8;
                let n = out.count as u64;
                // SAFETY: the caller's contract: a pointer to the object the
                // size names.
                unsafe {
                    match size {
                        Size::Hh => p.write_unaligned(n as u8),
                        Size::H => (p as *mut u16).write_unaligned(n as u16),
                        Size::None => (p as *mut u32).write_unaligned(n as u32),
                        _ => (p as *mut u64).write_unaligned(n),
                    }
                }
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => {
                let v = if size == Size::BigL { args.long_double() } else { args.double() };
                float(&mut out, spec, conv, v);
            }
            _ => return Err(Error::Invalid),
        }
    }
    Ok(out.count)
}

fn number(fmt: &[u8], i: &mut usize) -> usize {
    let mut n = 0usize;
    while let Some(&c) = fmt.get(*i) {
        if !c.is_ascii_digit() {
            break;
        }
        n = n.saturating_mul(10).saturating_add(usize::from(c - b'0'));
        *i += 1;
    }
    n
}

fn signed(v: u64, size: Size) -> i64 {
    match size {
        Size::Hh => i64::from(v as i8),
        Size::H => i64::from(v as i16),
        Size::None => i64::from(v as i32),
        _ => v as i64,
    }
}

fn unsigned(v: u64, size: Size) -> u64 {
    match size {
        Size::Hh => u64::from(v as u8),
        Size::H => u64::from(v as u16),
        Size::None => u64::from(v as u32),
        _ => v,
    }
}

/// The digits of `v` at the end of `buf`; how many.
fn digits_of(mut v: u64, base: u64, upper: bool, buf: &mut [u8; 24]) -> usize {
    let set: &[u8; 16] = if upper { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
    let mut n = 0;
    loop {
        n += 1;
        buf[24 - n] = set[(v % base) as usize];
        v /= base;
        if v == 0 {
            return n;
        }
    }
}

/// An integer conversion: sign, prefix (`0x`), then at least `precision`
/// digits (none for a zero with precision 0), padded to the width.
fn integer<S: Sink>(out: &mut Out<'_, S>, spec: Spec, sign: &[u8], prefix: &[u8], v: u64, base: u64, upper: bool) {
    let mut buf = [0u8; 24];
    let n = digits_of(v, base, upper, &mut buf);
    let digits: &[u8] = if v == 0 && spec.precision == Some(0) { &[] } else { &buf[24 - n..] };
    let mut zeros = spec.precision.map_or(0, |p| p.saturating_sub(digits.len()));
    // `%#o`: the precision grows, if needed, so the first digit is a zero
    // (a zero value with precision 0 prints one `0`).
    if base == 8 && spec.alt && zeros == 0 && digits.first() != Some(&b'0') {
        zeros = 1;
    }
    // The '0' flag is ignored when a precision is given (C11 7.21.6.1p6).
    let spec = Spec { zero: spec.zero && spec.precision.is_none(), ..spec };
    field(out, spec, sign, prefix, digits, zeros);
}

/// `sign prefix [zeros] body`, padded to the width: spaces on the left
/// (or right with '-'), or zeros after the prefix with '0'.
fn field<S: Sink>(out: &mut Out<'_, S>, spec: Spec, sign: &[u8], prefix: &[u8], body: &[u8], zeros: usize) {
    let len = sign.len() + prefix.len() + zeros + body.len();
    let pad = spec.width.saturating_sub(len);
    if !spec.left && !spec.zero {
        out.pad(b' ', pad);
    }
    out.put(sign);
    out.put(prefix);
    if !spec.left && spec.zero {
        out.pad(b'0', pad);
    }
    out.pad(b'0', zeros);
    out.put(body);
    if spec.left {
        out.pad(b' ', pad);
    }
}

/// A wide character in the C locale: ASCII only.
fn narrow(c: u32) -> Result<u8, Error> {
    if c < 0x80 { Ok(c as u8) } else { Err(Error::Encoding) }
}

/// The bytes at `p` up to its NUL, or `max` bytes, whichever comes first.
///
/// # Safety
/// `p` points to a string, or to at least `max` readable bytes.
unsafe fn bytes_until_nul<'a>(p: *const u8, max: Option<usize>) -> &'a [u8] {
    let mut n = 0;
    // SAFETY: the caller's contract.
    while max.is_none_or(|m| n < m) && unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: the `n` bytes just read.
    unsafe { core::slice::from_raw_parts(p, n) }
}

/// `%ls`: the wide string converted (ASCII only), at most `precision`
/// bytes.
///
/// # Safety
/// `p` is null or points to a wide string (or to as many characters as
/// the precision allows).
unsafe fn wide_string<S: Sink>(out: &mut Out<'_, S>, spec: Spec, p: *const u32) -> Result<(), Error> {
    if p.is_null() {
        let s: &[u8] = b"(null)";
        let s = &s[..spec.precision.map_or(s.len(), |n| n.min(s.len()))];
        field(out, spec, &[], &[], s, 0);
        return Ok(());
    }
    let mut n = 0;
    // SAFETY: the caller's contract; every character is checked before
    // the output is written.
    while spec.precision.is_none_or(|m| n < m) && unsafe { *p.add(n) } != 0 {
        // SAFETY: as above.
        narrow(unsafe { *p.add(n) })?;
        n += 1;
    }
    let pad = spec.width.saturating_sub(n);
    if !spec.left {
        out.pad(b' ', pad);
    }
    for k in 0..n {
        // SAFETY: checked above.
        out.put(&[unsafe { *p.add(k) } as u8]);
    }
    if spec.left {
        out.pad(b' ', pad);
    }
    Ok(())
}

/// A floating-point conversion.
fn float<S: Sink>(out: &mut Out<'_, S>, spec: Spec, conv: u8, v: f64) {
    let upper = conv.is_ascii_uppercase();
    let conv = conv.to_ascii_lowercase();
    let sign: &[u8] = if v.is_sign_negative() {
        b"-"
    } else if spec.plus {
        b"+"
    } else if spec.space {
        b" "
    } else {
        b""
    };
    if !v.is_finite() {
        let body: &[u8] = match (v.is_nan(), upper) {
            (true, false) => b"nan",
            (true, true) => b"NAN",
            (false, false) => b"inf",
            (false, true) => b"INF",
        };
        field(out, Spec { zero: false, ..spec }, sign, &[], body, 0);
        return;
    }
    let v = v.abs();
    let prefix: &[u8] = match (conv, upper) {
        (b'a', false) => b"0x",
        (b'a', true) => b"0X",
        _ => b"",
    };
    let body = Body { conv, upper, alt: spec.alt, precision: spec.precision, v };
    let mut count = Count(0);
    body.emit(&mut count);
    let len = sign.len() + prefix.len() + count.0;
    let pad = spec.width.saturating_sub(len);
    if !spec.left && !spec.zero {
        out.pad(b' ', pad);
    }
    out.put(sign);
    out.put(prefix);
    if !spec.left && spec.zero {
        out.pad(b'0', pad);
    }
    let mut sink = Fwd(out);
    body.emit(&mut sink);
    if spec.left {
        out.pad(b' ', pad);
    }
}

struct Fwd<'a, 'b, S>(&'a mut Out<'b, S>);

impl<S: Sink> Sink for Fwd<'_, '_, S> {
    fn put(&mut self, bytes: &[u8]) {
        self.0.put(bytes);
    }
}

/// The digits of a finite, non-negative value, without sign or `0x`.
struct Body {
    conv: u8,
    upper: bool,
    alt: bool,
    precision: Option<usize>,
    v: f64,
}

impl Body {
    fn emit(&self, sink: &mut dyn Sink) {
        let p = self.precision.unwrap_or(6);
        match self.conv {
            b'f' => fixed(sink, self.v, p, self.alt),
            b'e' => {
                let mut e = Exp::new(sink, self.upper, self.alt && p == 0);
                let _ = write!(e, "{:.*e}", p, self.v);
                e.finish();
            }
            b'g' => {
                let p = if p == 0 { 1 } else { p };
                let x = exponent(self.v, p - 1);
                let mut strip = Strip::new(sink, !self.alt);
                if (x as i64) < p as i64 && x >= -4 {
                    fixed(&mut strip, self.v, (p as i64 - 1 - x as i64) as usize, self.alt);
                    strip.finish();
                } else {
                    let mut e = Exp::new(&mut strip, self.upper, self.alt && p == 1);
                    let _ = write!(e, "{:.*e}", p - 1, self.v);
                    e.finish();
                    strip.finish();
                }
            }
            _ => hex(sink, self.v, self.precision, self.upper, self.alt),
        }
    }
}

/// `%f`'s digits: Rust's fixed notation is C's for finite values.
fn fixed(sink: &mut dyn Sink, v: f64, p: usize, alt: bool) {
    let mut w = Plain(sink);
    let _ = write!(w, "{v:.p$}");
    if p == 0 && alt {
        w.0.put(b".");
    }
}

struct Plain<'a>(&'a mut dyn Sink);

impl core::fmt::Write for Plain<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.put(s.as_bytes());
        Ok(())
    }
}

/// The decimal exponent of `v` written with `p` digits after the point.
fn exponent(v: f64, p: usize) -> i32 {
    struct Scan {
        seen_e: bool,
        neg: bool,
        exp: i32,
    }
    impl core::fmt::Write for Scan {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for c in s.bytes() {
                if !self.seen_e {
                    self.seen_e = c == b'e';
                } else if c == b'-' {
                    self.neg = true;
                } else if c.is_ascii_digit() {
                    self.exp = self.exp * 10 + i32::from(c - b'0');
                }
            }
            Ok(())
        }
    }
    let mut s = Scan { seen_e: false, neg: false, exp: 0 };
    let _ = write!(s, "{v:.p$e}");
    if s.neg { -s.exp } else { s.exp }
}

/// Rust's `1.5e3` as C's `1.5e+03`: the mantissa as is (with a point added
/// before the `e` when `point` is set), then the exponent with its sign
/// and at least two digits.
struct Exp<'a> {
    sink: &'a mut dyn Sink,
    upper: bool,
    point: bool,
    in_exp: bool,
    neg: bool,
    exp: u32,
}

impl<'a> Exp<'a> {
    fn new(sink: &'a mut dyn Sink, upper: bool, point: bool) -> Self {
        Exp { sink, upper, point, in_exp: false, neg: false, exp: 0 }
    }

    fn finish(self) {
        let mut buf = [0u8; 24];
        let n = digits_of(u64::from(self.exp), 10, false, &mut buf);
        let mut head = [0u8; 3];
        head[0] = if self.upper { b'E' } else { b'e' };
        head[1] = if self.neg { b'-' } else { b'+' };
        self.sink.put(&head[..2]);
        if n < 2 {
            self.sink.put(b"0");
        }
        self.sink.put(&buf[24 - n..]);
    }
}

impl core::fmt::Write for Exp<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.in_exp {
                if c == b'-' {
                    self.neg = true;
                } else if c.is_ascii_digit() {
                    self.exp = self.exp * 10 + u32::from(c - b'0');
                }
            } else if c == b'e' {
                self.in_exp = true;
                if self.point {
                    self.sink.put(b".");
                }
            } else {
                self.sink.put(&[c]);
            }
        }
        Ok(())
    }
}

/// `%g` without `#`: trailing zeros of the fraction are dropped, and the
/// point if nothing follows it. Zeros are held back until a non-zero digit
/// shows they are not trailing; an exponent ends the fraction.
struct Strip<'a> {
    sink: &'a mut dyn Sink,
    on: bool,
    in_fraction: bool,
    point: bool,
    zeros: usize,
}

impl<'a> Strip<'a> {
    fn new(sink: &'a mut dyn Sink, on: bool) -> Self {
        Strip { sink, on, in_fraction: false, point: false, zeros: 0 }
    }

    fn finish(&mut self) {
        self.in_fraction = false;
        self.point = false;
        self.zeros = 0;
    }
}

impl Sink for Strip<'_> {
    fn put(&mut self, bytes: &[u8]) {
        if !self.on {
            self.sink.put(bytes);
            return;
        }
        for &c in bytes {
            if !self.in_fraction {
                if c == b'.' {
                    self.in_fraction = true;
                    self.point = true;
                } else {
                    self.sink.put(&[c]);
                }
            } else if c == b'0' {
                self.zeros += 1;
            } else if c.is_ascii_digit() {
                if self.point {
                    self.sink.put(b".");
                    self.point = false;
                }
                for _ in 0..self.zeros {
                    self.sink.put(b"0");
                }
                self.zeros = 0;
                self.sink.put(&[c]);
            } else {
                // The exponent: the fraction has ended.
                self.in_fraction = false;
                self.point = false;
                self.zeros = 0;
                self.sink.put(&[c]);
            }
        }
    }
}

/// `%a`'s digits after the `0x`: `h.hhhp±d`.
fn hex(sink: &mut dyn Sink, v: f64, precision: Option<usize>, upper: bool, alt: bool) {
    let bits = v.to_bits();
    let mut e = ((bits >> 52) & 0x7ff) as i32;
    let mut m = bits & ((1 << 52) - 1);
    let mut lead = 1u64;
    if e == 0 {
        if m == 0 {
            lead = 0;
            e = 1023;
        } else {
            // Subnormal: shift until the leading 1 is the integer bit.
            e = 1;
            while m & (1 << 52) == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= (1 << 52) - 1;
        }
    }
    let mut exp = e - 1023;
    let digits = match precision {
        Some(p) if p < 13 => {
            // Round to p hexadecimal digits, ties to even.
            let drop = 4 * (13 - p) as u32;
            let full = (lead << 52) | m;
            let rem = full & ((1 << drop) - 1);
            let half = 1u64 << (drop - 1);
            let mut kept = full >> drop;
            if rem > half || (rem == half && kept & 1 == 1) {
                kept += 1;
            }
            if kept >> (4 * p) >= 2 {
                // 1.fff… rounded up to 2: renormalized as 1.000… × 2.
                kept >>= 1;
                exp += 1;
            }
            lead = kept >> (4 * p);
            m = kept & ((1u64 << (4 * p)) - 1);
            p
        }
        Some(p) => p,
        None => {
            let mut n = 13;
            while n > 0 && (m >> (4 * (13 - n))) & 0xf == 0 {
                n -= 1;
            }
            // `m` is aligned to 13 digits: drop the zeros at its end.
            m >>= 4 * (13 - n);
            n
        }
    };
    let set: &[u8; 16] = if upper { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
    sink.put(&[set[lead as usize]]);
    if digits > 0 || alt {
        sink.put(b".");
    }
    let shown = digits.min(13).min(match precision {
        Some(p) if p < 13 => p,
        _ => digits,
    });
    for k in (0..shown).rev() {
        sink.put(&[set[((m >> (4 * k)) & 0xf) as usize]]);
    }
    for _ in shown..digits {
        sink.put(b"0");
    }
    let mut buf = [0u8; 24];
    let n = digits_of(u64::from(exp.unsigned_abs()), 10, false, &mut buf);
    sink.put(if upper { b"P" } else { b"p" });
    sink.put(if exp < 0 { b"-" } else { b"+" });
    sink.put(&buf[24 - n..]);
}

#[cfg(test)]
mod tests;
