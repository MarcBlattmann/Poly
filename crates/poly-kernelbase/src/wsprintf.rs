//! `wsprintfA` / `wsprintfW` (Learn: "wsprintfW function", "Format specification fields";
//! `winuser.h`): the formatter of `user32`'s printf, which is not the C runtime's.
//!
//! A specification is `%[-][#][0][width][.precision][h|l]type` (`width` and `precision` digits or
//! `*`, which takes an `int` argument). Types: `c`/`C` (a character, the `A` form's ANSI and the `W`
//! form's wide, `C` the other), `d`/`i` (signed 32-bit), `u`, `x`, `X` (unsigned, with `#` a `0x`
//! prefix), `s`/`S` (a string, the same pairing as `c`/`C`, `precision` the most characters, NULL
//! is `(null)`), `p` (a pointer in upper-case hexadecimal, a pointer's width), and `%%`. `h` makes
//! `c`, `s` ANSI and `l` wide whichever the function. The result is cut to 1023 characters (the
//! buffer is 1024 bytes).
//!
//! Arguments are the machine words the call pushed (32-bit: 4-byte slots; 64-bit: the register and
//! stack slots), so a 64-bit value is not part of the format language.

use alloc::vec::Vec;

/// The most characters `wsprintf` writes, without the NUL.
pub const MAX_OUTPUT: usize = 1023;

/// The size of a pointer in the process, for `%p`.
pub const fn pointer_digits() -> usize {
    core::mem::size_of::<usize>() * 2
}

/// Format `fmt` (UTF-16 units; the `A` form widens its bytes first) with `args`. `wide_form` is
/// the `W` function (`%s` is wide); `read_string(pointer, wide)` reads the NUL-terminated string
/// an argument points to (units of 16 bits when `wide`, else bytes widened one to one).
pub fn format(
    fmt: &[u16],
    wide_form: bool,
    args: &[usize],
    pointer_width: usize,
    read_string: &dyn Fn(usize, bool) -> Vec<u16>,
) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    let mut next = 0usize;
    let take = |next: &mut usize| {
        let v = args.get(*next).copied().unwrap_or(0);
        *next += 1;
        v
    };
    let mut i = 0usize;
    while i < fmt.len() && fmt[i] != 0 && out.len() < MAX_OUTPUT {
        let c = fmt[i];
        i += 1;
        if c != u16::from(b'%') {
            out.push(c);
            continue;
        }
        let (mut left, mut alt, mut zero) = (false, false, false);
        while i < fmt.len() {
            match fmt[i] {
                0x2d => left = true,
                0x23 => alt = true,
                0x30 => zero = true,
                _ => break,
            }
            i += 1;
        }
        let mut width = 0usize;
        if fmt.get(i) == Some(&u16::from(b'*')) {
            let w = take(&mut next) as u32 as i32;
            if w < 0 {
                left = true;
            }
            width = w.unsigned_abs() as usize;
            i += 1;
        } else {
            while let Some(d) = fmt.get(i).and_then(|&u| digit(u)) {
                width = width.saturating_mul(10).saturating_add(d);
                i += 1;
            }
        }
        let mut precision: Option<usize> = None;
        if fmt.get(i) == Some(&u16::from(b'.')) {
            i += 1;
            if fmt.get(i) == Some(&u16::from(b'*')) {
                let p = take(&mut next) as u32 as i32;
                precision = (p >= 0).then_some(p as usize);
                i += 1;
            } else {
                let mut p = 0usize;
                while let Some(d) = fmt.get(i).and_then(|&u| digit(u)) {
                    p = p.saturating_mul(10).saturating_add(d);
                    i += 1;
                }
                precision = Some(p);
            }
        }
        let mut size = 0u8; // b'h' or b'l'
        while let Some(&u) = fmt.get(i) {
            match u {
                0x68 => size = b'h',
                0x6c => size = b'l',
                0x49 | 0x77 | 0x4c | 0x7a | 0x74 | 0x6a => {} // I, w, L, z, t, j: no width of their own
                _ => break,
            }
            i += 1;
        }
        let Some(&kind) = fmt.get(i) else { break };
        i += 1;
        if kind == 0 {
            break;
        }
        let kind = if kind < 0x80 { kind as u8 } else { 0 };
        let text: Vec<u16> = match kind {
            b'%' => alloc::vec![u16::from(b'%')],
            b'c' | b'C' => {
                // `c` is the function's own width, `C` the other; `h` and `l` say it outright.
                let wide = match size {
                    b'l' => true,
                    b'h' => false,
                    _ => wide_form == (kind == b'c'),
                };
                let v = take(&mut next);
                alloc::vec![if wide { v as u16 } else { crate::text::from_ansi(&[v as u8])[0] }]
            }
            b's' | b'S' => {
                let wide = match size {
                    b'l' => true,
                    b'h' => false,
                    _ => wide_form == (kind == b's'),
                };
                let p = take(&mut next);
                let mut s: Vec<u16> = if p == 0 { "(null)".encode_utf16().collect() } else { read_string(p, wide) };
                if let Some(n) = precision {
                    s.truncate(n);
                }
                s
            }
            b'd' | b'i' => {
                let v = take(&mut next) as u32 as i32;
                number(i64::from(v).unsigned_abs(), v < 0, 10, false, false, precision)
            }
            b'u' => number(u64::from(take(&mut next) as u32), false, 10, false, false, precision),
            b'x' | b'X' => {
                let v = u64::from(take(&mut next) as u32);
                number(v, false, 16, kind == b'X', alt && v != 0, precision)
            }
            b'p' => {
                let v = take(&mut next) as u64;
                let digits = pointer_width * 2;
                let mut s: Vec<u16> = alloc::format!("{v:0digits$X}").encode_utf16().collect();
                if alt {
                    s.splice(0..0, "0x".encode_utf16());
                }
                s
            }
            // An unknown type: the specification is copied as text (its type character at least).
            other => alloc::vec![u16::from(other)],
        };
        let numeric = matches!(kind, b'd' | b'i' | b'u' | b'x' | b'X');
        let pad = width.saturating_sub(text.len());
        if left {
            out.extend_from_slice(&text);
            out.extend(core::iter::repeat_n(u16::from(b' '), pad));
        } else if zero && numeric && precision.is_none() {
            // Zeros go after the sign and the prefix.
            let sign = usize::from(text.first() == Some(&u16::from(b'-')));
            let prefix = if text.len() >= sign + 2 && text[sign] == 0x30 && matches!(text[sign + 1], 0x78 | 0x58) {
                2
            } else {
                0
            };
            let at = sign + prefix;
            out.extend_from_slice(&text[..at]);
            out.extend(core::iter::repeat_n(u16::from(b'0'), pad));
            out.extend_from_slice(&text[at..]);
        } else {
            out.extend(core::iter::repeat_n(if zero && !numeric { 0x30 } else { 0x20 }, pad));
            out.extend_from_slice(&text);
        }
    }
    out.truncate(MAX_OUTPUT);
    out
}

fn digit(u: u16) -> Option<usize> {
    (0x30..=0x39).contains(&u).then(|| usize::from(u - 0x30))
}

/// An integer in `radix`, with the precision as a minimum number of digits.
fn number(v: u64, negative: bool, radix: u32, upper: bool, prefix: bool, precision: Option<usize>) -> Vec<u16> {
    let mut digits: Vec<u16> = Vec::new();
    let mut n = v;
    loop {
        let d = (n % u64::from(radix)) as u32;
        let ch = core::char::from_digit(d, radix).unwrap_or('0');
        digits.push(u16::from(if upper { ch.to_ascii_uppercase() } else { ch } as u8));
        n /= u64::from(radix);
        if n == 0 {
            break;
        }
    }
    while precision.is_some_and(|p| digits.len() < p) {
        digits.push(0x30);
    }
    if prefix {
        digits.push(u16::from(if upper { b'X' } else { b'x' }));
        digits.push(0x30);
    }
    if negative {
        digits.push(u16::from(b'-'));
    }
    digits.reverse();
    digits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn run(fmt: &str, wide_form: bool, args: &[usize]) -> alloc::string::String {
        // Strings are "Hello" at 100 (ANSI) and "Wide" at 200 (UTF-16).
        let read = |p: usize, wide: bool| match (p, wide) {
            (100, false) => w("Hello"),
            (200, true) => w("Wide"),
            (100, true) | (200, false) => w("?wrong width"),
            _ => w("other"),
        };
        alloc::string::String::from_utf16(&format(&w(fmt), wide_form, args, 4, &read)).unwrap()
    }

    #[test]
    fn integers_and_flags() {
        assert_eq!(run("%d|%i|%u", true, &[(-5i32) as u32 as usize, 7, (-1i32) as u32 as usize]), "-5|7|4294967295");
        assert_eq!(run("%x %X %#x %#X", true, &[255, 255, 255, 255]), "ff FF 0xff 0XFF");
        assert_eq!(run("%5d|%-5d|%05d|%.3d", true, &[42, 42, 42, 7]), "   42|42   |00042|007");
        assert_eq!(run("%05d", true, &[(-42i32) as u32 as usize]), "-0042");
        assert_eq!(run("%#06x", true, &[255]), "0x00ff");
        assert_eq!(run("%ld %lu %lx", true, &[1, 2, 255]), "1 2 ff");
        assert_eq!(run("%*d|%-*d|", true, &[4, 9, 4, 9]), "   9|9   |");
        assert_eq!(run("100%%", true, &[]), "100%");
    }

    #[test]
    fn strings_follow_the_function_width() {
        assert_eq!(run("%s", false, &[100]), "Hello", "wsprintfA: %s is ANSI");
        assert_eq!(run("%s", true, &[200]), "Wide", "wsprintfW: %s is wide");
        assert_eq!(run("%S", true, &[100]), "Hello", "wsprintfW: %S is ANSI");
        assert_eq!(run("%S", false, &[200]), "Wide");
        assert_eq!(run("%hs %ls", true, &[100, 200]), "Hello Wide");
        assert_eq!(run("%.3s|%8s|%-8s|", false, &[100, 100, 100]), "Hel|   Hello|Hello   |");
        assert_eq!(run("%s", true, &[0]), "(null)");
    }

    #[test]
    fn characters_pointers_and_odd_input() {
        assert_eq!(run("%c%C", true, &[0x41, 0x42]), "AB");
        assert_eq!(run("%c", false, &[0xE9]), "\u{e9}", "wsprintfA widens an ANSI character");
        assert_eq!(run("%p", true, &[0xBEEF]), "0000BEEF");
        assert_eq!(run("%q", true, &[]), "q", "an unknown type is kept");
        assert_eq!(run("no args %d", true, &[]), "no args 0", "a missing argument reads 0");
        assert_eq!(run("tail %", true, &[]), "tail ");
    }

    #[test]
    fn the_output_is_cut_at_the_buffer() {
        let long = "x".repeat(2000);
        assert_eq!(run(&long, true, &[]).len(), MAX_OUTPUT);
    }
}
