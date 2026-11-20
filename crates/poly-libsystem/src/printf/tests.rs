#![allow(clippy::approx_constant, clippy::manual_c_str_literals)]

use super::*;
use alloc::string::String;
use alloc::vec::Vec;

impl Sink for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

fn f(fmt: &str, ints: &[u64], doubles: &[f64]) -> String {
    let mut out = Vec::new();
    // SAFETY: the tests pass the arguments their formats name; no strings.
    let n = unsafe { format(fmt.as_bytes(), &mut Slots { ints, doubles }, &mut out) }.unwrap();
    assert_eq!(n, out.len());
    String::from_utf8(out).unwrap()
}

fn i(fmt: &str, v: i64) -> String {
    f(fmt, &[v as u64], &[])
}

fn d(fmt: &str, v: f64) -> String {
    f(fmt, &[], &[v])
}

#[test]
fn plain_text_and_percent() {
    assert_eq!(f("hello, world\n", &[], &[]), "hello, world\n");
    assert_eq!(f("100%%", &[], &[]), "100%");
    assert_eq!(f("", &[], &[]), "");
}

#[test]
fn signed_integers() {
    assert_eq!(i("%d", 42), "42");
    assert_eq!(i("%d", -42), "-42");
    assert_eq!(i("%i", 0), "0");
    assert_eq!(i("%5d", 42), "   42");
    assert_eq!(i("%-5d|", 42), "42   |");
    assert_eq!(i("%05d", -42), "-0042");
    assert_eq!(i("%+d", 42), "+42");
    assert_eq!(i("% d", 42), " 42");
    assert_eq!(i("%.5d", 42), "00042");
    assert_eq!(i("%08.5d", 42), "   00042");
    assert_eq!(i("%.0d", 0), "");
    assert_eq!(i("%5.0d|", 0), "     |");
    // int is 32 bits: the upper half of the slot is not read.
    assert_eq!(f("%d", &[0x1_0000_0005], &[]), "5");
    assert_eq!(f("%d", &[0xffff_ffff], &[]), "-1");
    assert_eq!(i("%ld", i64::MIN), "-9223372036854775808");
    assert_eq!(i("%lld", -1), "-1");
    assert_eq!(i("%qd", -1), "-1");
    assert_eq!(i("%hhd", 0x1ff), "-1");
    assert_eq!(i("%hd", 0x18000), "-32768");
    assert_eq!(i("%zd", -3), "-3");
    assert_eq!(i("%jd", 1 << 40), "1099511627776");
    assert_eq!(i("%D", -7), "-7");
}

#[test]
fn unsigned_integers() {
    assert_eq!(i("%u", -1), "4294967295");
    assert_eq!(i("%lu", -1), "18446744073709551615");
    assert_eq!(i("%x", 255), "ff");
    assert_eq!(i("%X", 255), "FF");
    assert_eq!(i("%#x", 255), "0xff");
    assert_eq!(i("%#X", 255), "0XFF");
    assert_eq!(i("%#x", 0), "0");
    assert_eq!(i("%#010x", 255), "0x000000ff");
    assert_eq!(i("%o", 8), "10");
    assert_eq!(i("%#o", 8), "010");
    assert_eq!(i("%#o", 0), "0");
    assert_eq!(i("%#.0o", 0), "0");
    assert_eq!(i("%#.3o", 8), "010");
    assert_eq!(i("%#06o", 8), "000010");
    assert_eq!(i("%hhu", 0x1ff), "255");
    assert_eq!(i("%U", -1), "18446744073709551615");
    assert_eq!(i("%O", 8), "10");
}

#[test]
fn star_width_and_precision() {
    assert_eq!(f("%*d|", &[5, 42], &[]), "   42|");
    assert_eq!(f("%*d|", &[(-5i64) as u64, 42], &[]), "42   |");
    assert_eq!(f("%.*d", &[4, 42], &[]), "0042");
    // A negative precision is as if it were missing.
    assert_eq!(f("%.*d", &[(-1i64) as u64, 0], &[]), "0");
}

#[test]
fn characters_strings_and_pointers() {
    assert_eq!(i("%c", i64::from(b'A')), "A");
    assert_eq!(i("%3c|", i64::from(b'A')), "  A|");
    assert_eq!(i("%-3c|", i64::from(b'A')), "A  |");
    let s = b"hello\0";
    let p = s.as_ptr() as u64;
    assert_eq!(f("%s", &[p], &[]), "hello");
    assert_eq!(f("%.3s", &[p], &[]), "hel");
    assert_eq!(f("%8s|", &[p], &[]), "   hello|");
    assert_eq!(f("%-8s|", &[p], &[]), "hello   |");
    assert_eq!(f("%s", &[0], &[]), "(null)");
    // The precision bounds the read: no NUL needed.
    let raw = *b"abc";
    assert_eq!(f("%.2s", &[raw.as_ptr() as u64], &[]), "ab");
    assert_eq!(f("%p", &[0x1234], &[]), "0x1234");
    assert_eq!(f("%p", &[0], &[]), "0x0");
    assert_eq!(f("%10p|", &[0xabc], &[]), "     0xabc|");
    let w: [u32; 3] = [u32::from(b'h'), u32::from(b'i'), 0];
    assert_eq!(f("%ls", &[w.as_ptr() as u64], &[]), "hi");
    assert_eq!(f("%S|%4ls", &[w.as_ptr() as u64, w.as_ptr() as u64], &[]), "hi|  hi");
    assert_eq!(f("%lc", &[u64::from(b'x')], &[]), "x");
}

#[test]
fn errors_and_odd_formats() {
    let mut out = Vec::new();
    // Above 0xFF: EILSEQ (the snprintf sheet); up to 0xFF the byte itself.
    // SAFETY: the format names the one integer.
    assert_eq!(unsafe { format(b"%lc", &mut Slots { ints: &[0x20ac], doubles: &[] }, &mut out) }, Err(Error::Encoding));
    let mut bytes = Vec::new();
    // SAFETY: the format names the one integer.
    assert_eq!(unsafe { format(b"%lc", &mut Slots { ints: &[0xe9], doubles: &[] }, &mut bytes) }, Ok(1));
    assert_eq!(bytes, [0xe9]);
    let w: [u32; 2] = [0x100, 0];
    let mut wide = Slots { ints: &[w.as_ptr() as u64], doubles: &[] };
    // SAFETY: the one argument is a wide string.
    let r = unsafe { format(b"%ls", &mut wide, &mut out) };
    assert_eq!(r, Err(Error::Encoding));
    // Unknown conversions print their letter; %5% pads.
    assert_eq!(f("%b %m %y %@", &[], &[]), "b m y @");
    assert_eq!(f("[%5%]", &[], &[]), "[    %]");
    // A format that ends inside a conversion prints nothing more.
    assert_eq!(f("abc%", &[], &[]), "abc");
    assert_eq!(f("x%q", &[], &[]), "x");
    assert_eq!(f("y%z", &[], &[]), "y");
    // A number 0, or one beyond the table, is invalid.
    // SAFETY: no argument is read.
    assert_eq!(unsafe { format(b"%0$d", &mut Slots { ints: &[1], doubles: &[] }, &mut out) }, Err(Error::Invalid));
}

#[test]
fn positional_arguments() {
    assert_eq!(f("%2$d %1$d", &[1, 2], &[]), "2 1");
    assert_eq!(f("%1$s %1$s", &[b"ab\0".as_ptr() as u64], &[]), "ab ab");
    // After a numbered conversion an unnumbered one takes the next number.
    assert_eq!(f("%2$d %d", &[1, 2, 3], &[]), "2 3");
    // Classes are read in argument order: here int, double, int.
    assert_eq!(f("%3$d %2$.1f %1$d", &[10, 30], &[2.5]), "30 2.5 10");
    assert_eq!(f("%1$*2$d|", &[7, 4], &[]), "   7|");
    assert_eq!(f("%2$.*1$f", &[2], &[3.14159]), "3.14");
}

#[test]
fn long_double_hexadecimal() {
    // x86-64's 80-bit format as it is: 1.0 is 0x8p-3 (the snprintf sheet).
    assert_eq!(d("%La", 1.0), "0x8p-3");
    assert_eq!(d("%La", 0.0), "0x0p+0");
    assert_eq!(d("%La", -2.0), "-0x8p-2");
    assert_eq!(d("%LA", 255.0), "0XF.FP+4");
    assert_eq!(d("%.1La", 1.0), "0x8.0p-3");
    assert_eq!(d("%.0La", 1.75), "0xep-3");
    assert_eq!(d("%La", f64::INFINITY), "inf");
    assert_eq!(f64_to_x87(1.0), (1 << 63, 0x3fff));
    assert_eq!(x87_to_f64(f64_to_x87(f64::from_bits(1)).0, f64_to_x87(f64::from_bits(1)).1), f64::from_bits(1));
    assert_eq!(x87_to_f64(f64_to_x87(-123.456).0, f64_to_x87(-123.456).1), -123.456);
}

#[test]
fn count_with_n() {
    let mut n32: u32 = 0;
    let mut n8: u8 = 0;
    let mut n64: u64 = 0;
    let out = f("abc%n%hhn12%lln", &[&raw mut n32 as u64, &raw mut n8 as u64, &raw mut n64 as u64], &[]);
    assert_eq!(out, "abc12");
    assert_eq!((n32, n8, n64), (3, 3, 5));
}

#[test]
fn fixed_notation() {
    assert_eq!(d("%f", 1.5), "1.500000");
    assert_eq!(d("%f", -1.5), "-1.500000");
    assert_eq!(d("%.2f", 3.14159), "3.14");
    assert_eq!(d("%.0f", 2.5), "2");
    assert_eq!(d("%.0f", 3.5), "4");
    assert_eq!(d("%#.0f", 3.0), "3.");
    assert_eq!(d("%10.3f|", 3.14159), "     3.142|");
    assert_eq!(d("%-10.3f|", 3.14159), "3.142     |");
    assert_eq!(d("%010.3f", -3.14159), "-00003.142");
    assert_eq!(d("%+f", 0.0), "+0.000000");
    assert_eq!(d("%f", -0.0), "-0.000000");
    assert_eq!(d("%f", 1e20), "100000000000000000000.000000");
    assert_eq!(d("%.20f", 0.1), "0.10000000000000000555");
    assert_eq!(d("%F", 1.0), "1.000000");
}

#[test]
fn exponent_notation() {
    assert_eq!(d("%e", 1500.0), "1.500000e+03");
    assert_eq!(d("%E", 1500.0), "1.500000E+03");
    assert_eq!(d("%e", 0.0), "0.000000e+00");
    assert_eq!(d("%.2e", 1.0e-7), "1.00e-07");
    assert_eq!(d("%e", 1e300), "1.000000e+300");
    assert_eq!(d("%.0e", 12345.0), "1e+04");
    assert_eq!(d("%#.0e", 12345.0), "1.e+04");
    assert_eq!(d("%12.2e|", -1234.5), "   -1.23e+03|");
    assert_eq!(d("%012.2e", -1234.5), "-0001.23e+03");
}

#[test]
fn general_notation() {
    assert_eq!(d("%g", 100000.0), "100000");
    assert_eq!(d("%g", 1000000.0), "1e+06");
    assert_eq!(d("%g", 0.0001), "0.0001");
    assert_eq!(d("%g", 0.00001), "1e-05");
    assert_eq!(d("%g", 1.5), "1.5");
    assert_eq!(d("%g", 0.0), "0");
    assert_eq!(d("%g", 3.14159265), "3.14159");
    assert_eq!(d("%.3g", 3.14159265), "3.14");
    assert_eq!(d("%.0g", 3.7), "4");
    assert_eq!(d("%#g", 1.5), "1.50000");
    assert_eq!(d("%#g", 100000.0), "100000.");
    assert_eq!(d("%G", 1e-10), "1E-10");
    assert_eq!(d("%g", 123456789.0), "1.23457e+08");
    assert_eq!(d("%g", 1e100), "1e+100");
    assert_eq!(d("%10g|", 2.5), "       2.5|");
    // Rounding can move the exponent: 9.9999995 to 6 digits is 10.0000.
    assert_eq!(d("%g", 9.9999996), "10");
    assert_eq!(d("%g", 999999.5), "1e+06");
}

#[test]
fn hexadecimal_floats() {
    assert_eq!(d("%a", 1.0), "0x1p+0");
    assert_eq!(d("%a", 0.5), "0x1p-1");
    assert_eq!(d("%a", -2.0), "-0x1p+1");
    assert_eq!(d("%A", 255.0), "0X1.FEP+7");
    assert_eq!(d("%a", 0.0), "0x0p+0");
    assert_eq!(d("%a", 0.1), "0x1.999999999999ap-4");
    assert_eq!(d("%.2a", 0.1), "0x1.9ap-4");
    assert_eq!(d("%.0a", 1.5), "0x1p+1");
    assert_eq!(d("%.3a", 1.0), "0x1.000p+0");
    assert_eq!(d("%#a", 1.0), "0x1.p+0");
    assert_eq!(d("%a", f64::from_bits(1)), "0x1p-1074");
    assert_eq!(d("%a", f64::MIN_POSITIVE), "0x1p-1022");
    assert_eq!(d("%12a|", 1.0), "      0x1p+0|");
    assert_eq!(d("%012a", 1.0), "0x0000001p+0");
}

#[test]
fn infinities_and_nans() {
    assert_eq!(d("%f", f64::INFINITY), "inf");
    assert_eq!(d("%F", f64::NEG_INFINITY), "-INF");
    assert_eq!(d("%e", f64::NAN), "nan");
    assert_eq!(d("%G", f64::NAN), "NAN");
    assert_eq!(d("%+g", f64::INFINITY), "+inf");
    assert_eq!(d("%05f", f64::INFINITY), "  inf");
}

#[test]
fn long_double_arguments() {
    // %Lf reads the next long double: here from the same double slots.
    assert_eq!(d("%Lf", 2.25), "2.250000");
    assert_eq!(x87_to_f64(0x8000_0000_0000_0000, 0x3fff), 1.0);
    assert_eq!(x87_to_f64(0xc000_0000_0000_0000, 0xc000), -3.0);
    assert_eq!(x87_to_f64(0, 0), 0.0);
    assert!(x87_to_f64(0x8000_0000_0000_0000, 0x7fff).is_infinite());
    assert!(x87_to_f64(0xc000_0000_0000_0000, 0x7fff).is_nan());
    assert_eq!(x87_to_f64(0x8000_0000_0000_0000, 0x3fff - 1074), f64::from_bits(1));
    assert!(x87_to_f64(0x8000_0000_0000_0000, 0x3fff + 1024).is_infinite());
}

#[test]
fn mixed_arguments_keep_their_own_order() {
    assert_eq!(f("%d %f %s %g %c", &[1, b"x\0".as_ptr() as u64, u64::from(b'z')], &[2.0, 3.0]), "1 2.000000 x 3 z");
}

#[test]
fn a_system_v_va_list() {
    // A save area with rdi..r9 and xmm0..7, three integers and one double
    // already taken by fixed parameters, then the overflow area.
    let mut save = [0u8; 176];
    for k in 0..6u64 {
        save[k as usize * 8..k as usize * 8 + 8].copy_from_slice(&(100 + k).to_le_bytes());
    }
    for k in 0..8 {
        save[48 + k * 16..48 + k * 16 + 8].copy_from_slice(&(f64::from(k as u32) + 0.5).to_le_bytes());
    }
    let mut overflow = [0u64; 8];
    overflow[0] = 7; // the 4th variadic integer
    overflow[1] = 8.5f64.to_bits(); // the 8th double
    overflow[2] = 9;
    let mut tag = VaListTag {
        gp_offset: 24,
        fp_offset: 48 + 16,
        overflow_arg_area: overflow.as_mut_ptr() as *mut u8,
        reg_save_area: save.as_mut_ptr(),
    };
    let mut va = VaList(&raw mut tag);
    assert_eq!((va.int(), va.int(), va.int()), (103, 104, 105));
    assert_eq!(va.int(), 7);
    for k in 1..8 {
        assert_eq!(va.double(), f64::from(k) + 0.5);
    }
    assert_eq!(va.double(), 8.5);
    assert_eq!(va.int(), 9);
}

#[test]
fn snprintf_buffer_keeps_what_fits() {
    let mut buf = [0u8; 4];
    let mut b = Buffer { buf: &mut buf, len: 0 };
    // SAFETY: the format names the one integer.
    let n = unsafe { format(b"%d-%d", &mut Slots { ints: &[123, 456], doubles: &[] }, &mut b) }.unwrap();
    assert_eq!((n, b.len), (7, 4));
    assert_eq!(&buf, b"123-");
}

#[test]
fn apple_arm64_va_list_reads_8_byte_slots() {
    // printf("%d %s %.1f %Lf %c", 7, "x", 2.5, 1.25L, 'q') on arm64: five
    // slots, the long double as a double.
    let s = b"x\0";
    let slots: [u64; 5] = [7, s.as_ptr() as u64, 2.5f64.to_bits(), 1.25f64.to_bits(), u64::from(b'q')];
    let mut out = [0u8; 64];
    let mut sink = Buffer { buf: &mut out, len: 0 };
    // SAFETY: the `%s` argument points to a NUL-terminated string.
    let n = unsafe { format(b"%d %s %.1f %Lf %c", &mut StackSlots(slots.as_ptr().cast()), &mut sink) }.unwrap();
    assert_eq!(&out[..n], b"7 x 2.5 1.250000 q");
}

/// The examples `specs/macos/libsystem_c.dylib/snprintf.yaml` gives.
#[test]
fn the_snprintf_sheets_examples() {
    assert_eq!(f("%D %O %U", &[5, 8, 7], &[]), "5 10 7");
    assert_eq!(i("%hhd", 300), "44");
    assert_eq!(i("%hd", 70000), "4464");
    assert_eq!(i("%u", -1), "4294967295");
    assert_eq!(i("%#o", 8), "010");
    assert_eq!(i("%#x", 0), "0");
    assert_eq!(i("%.0d", 0), "");
    assert_eq!(i("%05d", -7), "-0007");
    assert_eq!(i("%'d", 1_234_567), "1234567");
    assert_eq!(f("%#p", &[0], &[]), "0x0");
    assert_eq!(f("%.2s|%10s", &[0, 0], &[]), "(n|    (null)");
    assert_eq!(d("%f", 1.5), "1.500000");
    assert_eq!(d("%f", -0.0), "-0.000000");
    assert_eq!(d("%e", 12345.678), "1.234568e+04");
    assert_eq!(d("%g", 0.0001), "0.0001");
    assert_eq!(d("%g", 0.00001), "1e-05");
    assert_eq!(d("%g", 123_456_789.0), "1.23457e+08");
    assert_eq!(d("%.1g", 150.0), "2e+02");
    assert_eq!(d("%.20f", 0.1), "0.10000000000000000555");
    assert_eq!((d("%.0f", 0.5), d("%.0f", 1.5), d("%.0f", 2.5)), ("0".into(), "2".into(), "2".into()));
    assert_eq!(d("%#.0f", 1.0), "1.");
    assert_eq!(d("%05f", f64::NEG_INFINITY), " -inf");
    assert_eq!(d("%+f", f64::INFINITY), "+inf");
    assert_eq!(d("%a", 1.0), "0x1p+0");
    assert_eq!(d("%A", 255.0), "0X1.FEP+7");
    assert_eq!(d("%.2a", 1.0), "0x1.00p+0");
    assert_eq!(d("%.400f", 1.0).len(), 402);
    assert_eq!(i("%1500d", 1).len(), 1500);
    let mut buf = [0xffu8; 4];
    let mut b = Buffer { buf: &mut buf, len: 0 };
    // SAFETY: the format names the one integer.
    let n = unsafe { format(b"%c", &mut Slots { ints: &[0], doubles: &[] }, &mut b) }.unwrap();
    assert_eq!((n, b.len, buf[0]), (1, 1, 0));
}
