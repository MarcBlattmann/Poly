// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::ctype<char> character functions (?is@?$ctype@D@std@@QEBA_NFD@Z, ?is@ (range), ?narrow@ (both), the inline scan_is, scan_not, tolower, toupper, widen and their do_ virtual functions): classification, case conversion, widening and narrowing in the C locale.
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
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
static void g2(poly::Test& t) {
    const std::ctype<char>& ct = std::use_facet<std::ctype<char>>(std::locale::classic());
    t.check("is(space, ' ')", ct.is(std::ctype_base::space, ' '), 0);
    t.check("is(space, 'a') is false", !ct.is(std::ctype_base::space, 'a'), 0);
    t.check("is(alpha, 'q')", ct.is(std::ctype_base::alpha, 'q'), 0);
    t.check("is(digit, '7')", ct.is(std::ctype_base::digit, '7'), 0);
    t.check("is(xdigit, 'f') and not 'g'", ct.is(std::ctype_base::xdigit, 'f') && !ct.is(std::ctype_base::xdigit, 'g'), 0);
    t.check("is(upper|lower, 'Q')", ct.is(std::ctype_base::upper | std::ctype_base::lower, 'Q'), 0);
    t.check("is(punct, '!')", ct.is(std::ctype_base::punct, '!'), 0);
    t.check("is(cntrl, '\\t')", ct.is(std::ctype_base::cntrl, '\t'), 0);
    t.check("is(blank, space) is true (blank is 0x48: the blank bit and the space bit)", ct.is(std::ctype_base::blank, 32) && ct.is(std::ctype_base::blank, 10), 0);
    t.check("a high byte has no class in the C locale", !ct.is(std::ctype_base::alpha | std::ctype_base::print | std::ctype_base::punct, (char)0xE9), 0);
    unsigned short masks[6];
    static const char sample[] = "a1 !	";
    const char* r = ct.is(sample, sample + 5, (std::ctype_base::mask*)masks);
    t.check("is(range) returns the end", r != 0, 0);
    t.expect_hex("mask of 'a'", masks[0], (poly_u32)ct.table()['a']);
    t.expect_hex("mask of '1'", masks[1], (poly_u32)ct.table()['1']);
    t.expect_hex("mask of ' '", masks[2], (poly_u32)ct.table()[' ']);
    const char* text = "ab12 cd";
    t.expect("scan_is(digit) finds the first digit", ct.scan_is(std::ctype_base::digit, text, text + 7) - text, 2);
    t.expect("scan_is with no match returns the end", ct.scan_is(std::ctype_base::punct, text, text + 7) - text, 7);
    t.expect("scan_not(alpha) finds the first non-letter", ct.scan_not(std::ctype_base::alpha, text, text + 7) - text, 2);
    t.expect("scan_not at the start", ct.scan_not(std::ctype_base::digit, text, text + 7) - text, 0);
    t.expect("tolower('Q')", ct.tolower('Q'), 'q');
    t.expect("toupper('q')", ct.toupper('q'), 'Q');
    t.expect("tolower of a non-letter", ct.tolower('5'), '5');
    t.expect("toupper of a high byte is unchanged in the C locale", (unsigned char)ct.toupper((char)0xE9), 0xE9);
    t.expect("tolower of a high byte is unchanged", (unsigned char)ct.tolower((char)0xC9), 0xC9);
    char buf[16];
    memcpy(buf, "Hello, World", 13);
    const char* e = ct.toupper(buf, buf + 12);
    t.expect_str("toupper(range)", buf, "HELLO, WORLD");
    t.expect("returns the end", e - buf, 12);
    ct.tolower(buf, buf + 12);
    t.expect_str("tolower(range)", buf, "hello, world");
    t.expect("widen('x')", ct.widen('x'), 'x');
    t.expect("widen of a high byte", (unsigned char)ct.widen((char)0xFF), 0xFF);
    char out[16];
    static const char abc[] = "abc";
    e = ct.widen(abc, abc + 3, out);
    t.expect("widen(range) copies", out[0] + out[1] + out[2], 'a' + 'b' + 'c');
    t.expect("and returns the end of the input", (long long)(e != 0), 1);
    t.expect("narrow('x', '?')", ct.narrow('x', '?'), 'x');
    t.expect("narrow of a high byte keeps it", (unsigned char)ct.narrow((char)0xFE, '?'), 0xFE);
    static const char xyz[] = "xyz";
    e = ct.narrow(xyz, xyz + 3, '?', out);
    t.expect("narrow(range) copies", out[0] + out[1] + out[2], 'x' + 'y' + 'z');
}
static void test(poly::Test& t) {
    g2(t);
}
POLY_TEST_MAIN(test)
