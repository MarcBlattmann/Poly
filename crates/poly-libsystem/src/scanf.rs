//! The `scanf` family's engine (C11 7.21.6.2) over a string (`sscanf`,
//! `vsscanf`): white space in the format matches any amount of input white
//! space; other ordinary characters must match; conversions `d i u o x X
//! c s [ n p a e f g E G %` with field widths, `*` (no assignment) and the
//! sizes `hh h l ll j z t L q`. The destination pointers come from the
//! same [`Args`] as `printf`'s arguments.
//!
//! The result is the number of assignments, or `EOF` (-1) if the input
//! ended before the first conversion could match.

use crate::printf::Args;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Size {
    Char,
    Short,
    Int,
    Long,
    LongLong,
    LongDouble,
}

fn is_space(c: u8) -> bool {
    c == b' ' || (0x09..=0x0d).contains(&c)
}

/// An integer at the start of `s` with at most `width` bytes in `base` (0:
/// by prefix): (value, bytes used), `None` if no digit.
fn integer(s: &[u8], width: usize, base: u32) -> Option<(u64, usize)> {
    let s = &s[..s.len().min(width)];
    let mut i = 0;
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut base = base;
    let has_x = |i: usize| i + 1 < s.len() && s[i] == b'0' && (s[i + 1] | 0x20) == b'x';
    if (base == 0 || base == 16) && has_x(i) && s.get(i + 2).is_some_and(|c| c.is_ascii_hexdigit()) {
        i += 2;
        base = 16;
    } else if base == 0 {
        base = if s.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    let start = i;
    let mut v: u64 = 0;
    while i < s.len() {
        let Some(d) = (s[i] as char).to_digit(base) else { break };
        v = v.wrapping_mul(u64::from(base)).wrapping_add(u64::from(d));
        i += 1;
    }
    if i == start {
        return None;
    }
    Some((if neg { v.wrapping_neg() } else { v }, i))
}

/// Store an integer of `size` at `p`.
///
/// # Safety
/// `p` points to an object of that size.
unsafe fn store_int(p: *mut u8, size: Size, v: u64) {
    // SAFETY: the caller's contract.
    unsafe {
        match size {
            Size::Char => p.write(v as u8),
            Size::Short => (p as *mut u16).write_unaligned(v as u16),
            Size::Int => (p as *mut u32).write_unaligned(v as u32),
            _ => (p as *mut u64).write_unaligned(v),
        }
    }
}

/// `vsscanf(input, fmt, args)`.
///
/// # Safety
/// `args` yields one valid pointer per assigning conversion of `fmt`, to
/// an object of the type the conversion names.
pub unsafe fn sscanf(input: &[u8], fmt: &[u8], args: &mut dyn Args) -> i32 {
    let mut i = 0; // in input
    let mut f = 0; // in fmt
    let mut assigned = 0;
    let mut converted_any = false;
    let eof = |assigned: i32, any: bool| if any { assigned } else { -1 };
    while f < fmt.len() {
        let c = fmt[f];
        if is_space(c) {
            while f < fmt.len() && is_space(fmt[f]) {
                f += 1;
            }
            while i < input.len() && is_space(input[i]) {
                i += 1;
            }
            continue;
        }
        if c != b'%' {
            if i >= input.len() {
                return eof(assigned, converted_any);
            }
            if input[i] != c {
                return assigned;
            }
            i += 1;
            f += 1;
            continue;
        }
        f += 1;
        let suppress = fmt.get(f) == Some(&b'*');
        if suppress {
            f += 1;
        }
        let mut width = 0usize;
        while f < fmt.len() && fmt[f].is_ascii_digit() {
            width = width * 10 + usize::from(fmt[f] - b'0');
            f += 1;
        }
        let mut size = Size::Int;
        loop {
            match fmt.get(f) {
                Some(b'h') if size == Size::Short => size = Size::Char,
                Some(b'h') => size = Size::Short,
                Some(b'l') if size == Size::Long => size = Size::LongLong,
                Some(b'l') => size = Size::Long,
                Some(b'j' | b'z' | b't' | b'q') => size = Size::LongLong,
                Some(b'L') => size = Size::LongDouble,
                _ => break,
            }
            f += 1;
        }
        let Some(&conv) = fmt.get(f) else { return assigned };
        f += 1;
        if conv == b'%' {
            while i < input.len() && is_space(input[i]) {
                i += 1;
            }
            if i >= input.len() {
                return eof(assigned, converted_any);
            }
            if input[i] != b'%' {
                return assigned;
            }
            i += 1;
            continue;
        }
        if conv == b'n' {
            if !suppress {
                // SAFETY: the caller's contract.
                unsafe { store_int(args.int() as *mut u8, size, i as u64) };
            }
            continue;
        }
        // Every other conversion but %c and %[ skips white space first.
        if conv != b'c' && conv != b'[' {
            while i < input.len() && is_space(input[i]) {
                i += 1;
            }
        }
        if i >= input.len() {
            return eof(assigned, converted_any);
        }
        let rest = &input[i..];
        match conv {
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'p' => {
                let base = match conv {
                    b'd' | b'u' => 10,
                    b'i' => 0,
                    b'o' => 8,
                    _ => 16,
                };
                let w = if width == 0 { usize::MAX } else { width };
                let Some((v, n)) = integer(rest, w, base) else { return assigned };
                i += n;
                converted_any = true;
                if !suppress {
                    let size = if conv == b'p' { Size::Long } else { size };
                    // SAFETY: the caller's contract.
                    unsafe { store_int(args.int() as *mut u8, size, v) };
                    assigned += 1;
                }
            }
            b'a' | b'e' | b'f' | b'g' | b'A' | b'E' | b'F' | b'G' => {
                let w = if width == 0 { rest.len() } else { width.min(rest.len()) };
                let p = crate::num::strtod(&rest[..w]);
                if p.len == 0 {
                    return assigned;
                }
                i += p.len;
                converted_any = true;
                if !suppress {
                    let at = args.int() as *mut u8;
                    // SAFETY: the caller's contract: a float for no size,
                    // a double for l (and L, which is double on arm64; x86
                    // long double gets a double too, a v0 limit).
                    unsafe {
                        if size == Size::Int {
                            (at as *mut f32).write_unaligned(p.value as f32);
                        } else {
                            (at as *mut f64).write_unaligned(p.value);
                        }
                    }
                    assigned += 1;
                }
            }
            b's' | b'c' | b'[' => {
                let mut set = [false; 256];
                if conv == b'[' {
                    let neg = fmt.get(f) == Some(&b'^');
                    if neg {
                        f += 1;
                    }
                    let mut first = true;
                    while f < fmt.len() && (fmt[f] != b']' || first) {
                        if fmt[f] == b'-' && !first && f + 1 < fmt.len() && fmt[f + 1] != b']' {
                            let (lo, hi) = (fmt[f - 1], fmt[f + 1]);
                            for b in lo..=hi {
                                set[usize::from(b)] = true;
                            }
                            f += 2;
                        } else {
                            set[usize::from(fmt[f])] = true;
                            f += 1;
                        }
                        first = false;
                    }
                    f += 1; // the ']'
                    if neg {
                        for v in set.iter_mut() {
                            *v = !*v;
                        }
                    }
                }
                let w = if width == 0 { if conv == b'c' { 1 } else { usize::MAX } } else { width };
                let mut n = 0;
                while n < w && n < rest.len() {
                    let b = rest[n];
                    let ok = match conv {
                        b's' => !is_space(b),
                        b'[' => set[usize::from(b)],
                        _ => true,
                    };
                    if !ok {
                        break;
                    }
                    n += 1;
                }
                if n == 0 || (conv == b'c' && n < w) {
                    return assigned;
                }
                i += n;
                converted_any = true;
                if !suppress {
                    let at = args.int() as *mut u8;
                    // SAFETY: the caller's contract: room for n bytes, and
                    // the NUL for %s and %[.
                    unsafe {
                        core::ptr::copy_nonoverlapping(rest.as_ptr(), at, n);
                        if conv != b'c' {
                            at.add(n).write(0);
                        }
                    }
                    assigned += 1;
                }
            }
            _ => return assigned,
        }
    }
    assigned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printf::Slots;

    #[test]
    fn integers_and_literals() {
        let (mut n, mut pg) = (0i32, 0i32);
        let ptrs = [&raw mut n as u64, &raw mut pg as u64];
        // SAFETY: two int pointers for two %d.
        let r = unsafe {
            sscanf(b"| size 4096 pagesize 512", b"| size %d pagesize %d", &mut Slots { ints: &ptrs, doubles: &[] })
        };
        assert_eq!((r, n, pg), (2, 4096, 512));
        let mut v = [0u32; 4];
        let ptrs = v.each_mut().map(|p| p as *mut u32 as u64);
        // SAFETY: four int pointers for four conversions.
        let r = unsafe { sscanf(b"| 12: 0a ff 3", b"| %d: %x %x %x", &mut Slots { ints: &ptrs, doubles: &[] }) };
        assert_eq!((r, v), (4, [12, 10, 255, 3]));
        let mut x = 0i32;
        let ptrs = [&raw mut x as u64];
        // SAFETY: as above.
        assert_eq!(unsafe { sscanf(b"abc", b"%d", &mut Slots { ints: &ptrs, doubles: &[] }) }, 0);
        // SAFETY: as above.
        assert_eq!(unsafe { sscanf(b"   ", b"%d", &mut Slots { ints: &ptrs, doubles: &[] }) }, -1);
        // SAFETY: as above.
        assert_eq!(unsafe { sscanf(b"0x1f -010", b"%i", &mut Slots { ints: &ptrs, doubles: &[] }) }, 1);
        assert_eq!(x, 31);
        let mut l = 0i64;
        let mut h = 0i16;
        let mut used = 0i32;
        let ptrs = [&raw mut l as u64, &raw mut h as u64, &raw mut used as u64];
        // SAFETY: a long long, a short and an int.
        let r = unsafe { sscanf(b"-123456789012 -7 tail", b"%lld %hd%n", &mut Slots { ints: &ptrs, doubles: &[] }) };
        assert_eq!((r, l, h, used), (2, -123_456_789_012, -7, 16));
    }

    #[test]
    fn strings_sets_and_floats() {
        let mut s = [0u8; 16];
        let mut c = [0u8; 2];
        let mut set = [0u8; 16];
        let mut d = 0f64;
        let mut fl = 0f32;
        let ptrs = [
            s.as_mut_ptr() as u64,
            c.as_mut_ptr() as u64,
            set.as_mut_ptr() as u64,
            &raw mut d as u64,
            &raw mut fl as u64,
        ];
        // SAFETY: buffers large enough, a double and a float.
        let r = unsafe {
            sscanf(b"  word xyabc123 2.5e1 0.5", b"%s %2c%[a-c]%*d %lf %f", &mut Slots { ints: &ptrs, doubles: &[] })
        };
        assert_eq!(r, 5);
        assert_eq!(&s[..5], b"word\0");
        assert_eq!(&c, b"xy");
        assert_eq!(&set[..4], b"abc\0");
        assert_eq!((d, fl), (25.0, 0.5));
        let mut t = [0u8; 8];
        let ptrs = [t.as_mut_ptr() as u64];
        // SAFETY: room for the string.
        let r = unsafe { sscanf(b"a,b", b"%[^,]", &mut Slots { ints: &ptrs, doubles: &[] }) };
        assert_eq!((r, &t[..2]), (1, &b"a\0"[..]));
        // SAFETY: no conversions.
        assert_eq!(unsafe { sscanf(b"50%", b"50%%", &mut Slots { ints: &[], doubles: &[] }) }, 0);
    }
}
