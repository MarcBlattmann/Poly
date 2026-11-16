// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_istream<char>::operator>> for bool, short, unsigned short, int, unsigned, long, unsigned long, long long, unsigned long long, float, double, long double, void* (??5?$basic_istream@...) and _Ipfx: parsing, bases, overflow, errors and the state bits.
#include "poly_wtest.h"
#include <ostream>
#include <iomanip>
#include <sstream>
#include <istream>
#include <streambuf>
#include <locale>
#include <system_error>
#include <new>
#include <stdexcept>
void* __cdecl operator new(unsigned long long n) { return malloc(n); }
namespace std { void __cdecl _Facet_Register(_Facet_base*) {} }
struct IBuf : std::streambuf {
    char data[256]; int pos = 0, len = 0;
    IBuf(const char* s) { while (s[len]) { data[len] = s[len]; len++; } }
    int underflow() override { return pos < len ? (unsigned char)data[pos] : -1; }
    int uflow() override { return pos < len ? (unsigned char)data[pos++] : -1; }
};
static double from_bits(unsigned long long u) { double d; memcpy(&d, &u, 8); return d; }
template <class T> static void rd(poly::Test& t, const char* src, T want, int state, unsigned flagsOn = 0, unsigned flagsOff = 0) {
    IBuf sb(src);
    std::istream is(&sb);
    is.setf(flagsOn);
    is.unsetf(flagsOff);
    T v = (T)99;
    is >> v;
    poly::Line l;
    l.str("value read ").str(src).str(" => value ").dec((poly_i64)v).str(" state ").dec(is.rdstate());
    l.end();
    t.check(src, v == want && is.rdstate() == state, (poly_i64)v);
}
static void g1(poly::Test& t) {
    rd<int>(t, "42", 42, 2 - 1 + 0);               // reaches the end: eofbit
    rd<int>(t, "42 ", 42, 0);
    rd<int>(t, "  -17x", -17, 0);
    rd<int>(t, "+8 ", 8, 0);
    rd<int>(t, "0", 0, 1);
    rd<int>(t, "2147483647 ", 2147483647, 0);
    rd<int>(t, "2147483648 ", 2147483647, 2);       // out of range: failbit, max stored
    rd<int>(t, "-2147483648 ", -2147483647 - 1, 0);
    rd<int>(t, "-2147483649 ", -2147483647 - 1, 2);
    rd<int>(t, "abc", 0, 2);
    rd<int>(t, "", 99, 3);
    rd<int>(t, "   ", 99, 3);
    rd<int>(t, "-", 0, 3);
    rd<int>(t, "+", 0, 3);
    rd<unsigned>(t, "4294967295 ", 4294967295u, 0);
    rd<unsigned>(t, "4294967296 ", 4294967295u, 2);
    rd<unsigned>(t, "-1 ", 4294967295u, 0);
    rd<short>(t, "32767 ", 32767, 0);
    rd<short>(t, "32768 ", 32767, 2);
    rd<short>(t, "-32769 ", -32768, 2);
    rd<unsigned short>(t, "65535 ", 65535, 0);
    rd<unsigned short>(t, "65536 ", 65535, 2);
    rd<long>(t, "-5 ", -5, 0);
    rd<unsigned long>(t, "7 ", 7, 0);
    rd<long long>(t, "9223372036854775807 ", 9223372036854775807LL, 0);
    rd<long long>(t, "9223372036854775808 ", 9223372036854775807LL, 2);
    rd<unsigned long long>(t, "18446744073709551615 ", 18446744073709551615ULL, 0);
    rd<unsigned long long>(t, "18446744073709551616 ", 18446744073709551615ULL, 2);
}
static void g2(poly::Test& t) {
    rd<int>(t, "ff ", 255, 0, std::ios_base::hex, std::ios_base::dec);
    rd<int>(t, "FF ", 255, 0, std::ios_base::hex, std::ios_base::dec);
    rd<int>(t, "0xff ", 255, 0, std::ios_base::hex, std::ios_base::dec);
    rd<int>(t, "0Xff ", 255, 0, std::ios_base::hex, std::ios_base::dec);
    rd<int>(t, "17 ", 15, 0, std::ios_base::oct, std::ios_base::dec);
    rd<int>(t, "0x1f ", 31, 0, 0, std::ios_base::dec);            // basefield 0: base from the prefix
    rd<int>(t, "017 ", 15, 0, 0, std::ios_base::dec);
    rd<int>(t, "17 ", 17, 0, 0, std::ios_base::dec);
    rd<int>(t, "0x ", 0, 2, std::ios_base::hex, std::ios_base::dec);
    rd<int>(t, "12 34", 12, 0);
    rd<int>(t, "0x", 0, 3, 0, std::ios_base::dec);
    rd<int>(t, "08 ", 0, 0, 0, std::ios_base::dec);
    rd<bool>(t, "1 ", true, 0);
    rd<bool>(t, "0 ", false, 0);
    rd<bool>(t, "2 ", true, 2);
    rd<bool>(t, "true ", true, 0, std::ios_base::boolalpha);
    rd<bool>(t, "false ", false, 0, std::ios_base::boolalpha);
    rd<bool>(t, "tru ", false, 2, std::ios_base::boolalpha);
    rd<bool>(t, "1 ", false, 2, std::ios_base::boolalpha);
    rd<int>(t, "12", 12, 1, 0, std::ios_base::skipws);
    rd<int>(t, " 12", 0, 2, 0, std::ios_base::skipws);          // without skipws the blank is not a digit: failbit
}
static void g3(poly::Test& t) {
    rd<double>(t, "3.25 ", 3.25, 0);
    rd<double>(t, "-0.5 ", -0.5, 0);
    rd<double>(t, "1e3 ", 1000.0, 0);
    rd<double>(t, "1.5E-2 ", 0.015, 0);
    rd<double>(t, ".5 ", 0.5, 0);
    rd<double>(t, "5. ", 5.0, 0);
    rd<double>(t, "7", 7.0, 1);
    rd<double>(t, "abc", 0.0, 2);
    rd<double>(t, "1e ", 0.0, 2);
    rd<double>(t, "1e+ ", 0.0, 2);
    rd<float>(t, "2.5 ", 2.5f, 0);
    rd<long double>(t, "0.25 ", 0.25, 0);
    rd<double>(t, "1e999 ", from_bits(0x7FF0000000000000ULL), 2);
    rd<double>(t, "123456789012 ", 123456789012.0, 0);
    rd<double>(t, "0x10 ", 16.0, 0);
    rd<double>(t, "inf ", 0.0, 2);
    rd<double>(t, "nan ", 0.0, 2);
}
static void g4(poly::Test& t) {
    {
        IBuf sb("0000000000001234 ");
        std::istream is(&sb);
        void* p = 0;
        is >> p;
        t.expect("void* read as hex", (long long)p, 0x1234);
        t.expect("state", is.rdstate(), 0);
    }
    {
        // several values in a row, and the state after each
        IBuf sb("1 2 x 4");
        std::istream is(&sb);
        int a = 0, b = 0, c = 0, d = 0;
        is >> a >> b >> c >> d;
        t.expect("a", a, 1);
        t.expect("b", b, 2);
        t.expect("c (failed)", c, 0);
        t.expect("d is not read after a failure", d, 0);
        t.expect("failbit", is.rdstate(), 2);
        t.expect("the failed character stays in the buffer", sb.pos, 4);
    }
    {
        // exceptions
        IBuf sb("x");
        std::istream is(&sb);
        is.exceptions(std::ios_base::failbit);
        int a = 0;
        bool thrown = false;
        try { is >> a; } catch (const std::ios_base::failure&) { thrown = true; }
        t.check("failbit in the mask: >> throws ios_base::failure", thrown, 0);
    }
    {
        // the stream in a bad state does not read
        IBuf sb("5");
        std::istream is(&sb);
        is.setstate(std::ios_base::failbit);
        int a = 99;
        is >> a;
        t.expect("a failed stream leaves the value alone", a, 99);
        t.expect("and the buffer unread", sb.pos, 0);
    }
    {
        // a tied stream is flushed before input
        struct Fl : std::streambuf {
            int syncs = 0;
            int sync() override { syncs++; return 0; }
            int overflow(int c) override { return c; }
        };
        Fl fl;
        std::ostream tie(&fl);
        IBuf sb("5 ");
        std::istream is(&sb);
        is.tie(&tie);
        int a = 0;
        is >> a;
        t.expect("the tied output stream was flushed once", fl.syncs, 1);
        t.expect("value", a, 5);
    }
}
static void test(poly::Test& t) {
    g1(t);
    g2(t);
    g3(t);
    g4(t);
}
POLY_TEST_MAIN(test)
