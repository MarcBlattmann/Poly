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
fn errors() {
    let mut out = Vec::new();
    let mut none = Slots { ints: &[0x20ac], doubles: &[] };
    // SAFETY: the format names the one integer.
    assert_eq!(unsafe { format(b"%lc", &mut none, &mut out) }, Err(Error::Encoding));
    // SAFETY: as above.
    assert_eq!(unsafe { format(b"%1$d", &mut Slots { ints: &[1], doubles: &[] }, &mut out) }, Err(Error::Invalid));
    // SAFETY: no argument is read.
    assert_eq!(unsafe { format(b"%k", &mut Slots { ints: &[], doubles: &[] }, &mut out) }, Err(Error::Invalid));
    // SAFETY: as above.
    assert_eq!(unsafe { format(b"abc%", &mut Slots { ints: &[], doubles: &[] }, &mut out) }, Err(Error::Invalid));
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
