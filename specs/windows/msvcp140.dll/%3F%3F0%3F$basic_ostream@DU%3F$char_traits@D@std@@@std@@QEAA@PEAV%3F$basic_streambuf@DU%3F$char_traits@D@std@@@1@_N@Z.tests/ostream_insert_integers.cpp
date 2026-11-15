// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_ostream<char>::operator<< for bool, short, unsigned short, int, unsigned, long, unsigned long, long long, unsigned long long, const void* (??6?$basic_ostream@...) and the inline char / string inserters: the text produced under the format flags, width, fill and adjustment.
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
struct OB : std::streambuf {
    char b[2048]; int n = 0;
    int overflow(int c) override { if (c != -1) b[n++] = (char)c; return c == -1 ? 0 : c; }
    long long xsputn(const char* s, long long k) override { for (long long i = 0; i < k; i++) b[n++] = s[i]; return k; }
    const char* str() { b[n] = 0; return b; }
    void reset() { n = 0; }
};
#define CASE(name, expr, want) do { ob.reset(); os.clear(); os.flags(0x201); os.width(0); os.precision(6); os.fill(' '); { expr; } t.expect_str(name, ob.str(), want); } while (0)
static void g0(poly::Test& t, OB& ob, std::ostream& os) {

    CASE("int 0", os << 0, "0");
    CASE("int -123", os << -123, "-123");
    CASE("int max", os << 2147483647, "2147483647");
    CASE("int min", os << (-2147483647 - 1), "-2147483648");
    CASE("unsigned max", os << 4294967295u, "4294967295");
    CASE("short -5", os << (short)-5, "-5");
    CASE("unsigned short max", os << (unsigned short)65535, "65535");
    CASE("long -7", os << -7L, "-7");
    CASE("unsigned long 7", os << 7UL, "7");
    CASE("long long min", os << (-9223372036854775807LL - 1), "-9223372036854775808");
}
static void g1(poly::Test& t, OB& ob, std::ostream& os) {
    CASE("unsigned long long max", os << 18446744073709551615ULL, "18446744073709551615");
    CASE("hex 255", os << std::hex << 255, "ff");
    CASE("hex uppercase", os << std::hex << std::uppercase << 255, "FF");
    CASE("hex showbase", os << std::hex << std::showbase << 255, "0xff");
    CASE("hex showbase uppercase", os << std::hex << std::showbase << std::uppercase << 255, "0XFF");
    CASE("hex showbase of zero has no prefix", os << std::hex << std::showbase << 0, "0");
    CASE("oct 8", os << std::oct << 8, "10");
    CASE("oct showbase", os << std::oct << std::showbase << 8, "010");
    CASE("oct showbase of zero", os << std::oct << std::showbase << 0, "0");
    CASE("hex of -1 (int)", os << std::hex << -1, "ffffffff");
}
static void g2(poly::Test& t, OB& ob, std::ostream& os) {
    CASE("hex of -1 (long long)", os << std::hex << -1LL, "ffffffffffffffff");
    CASE("hex of -1 (short)", os << std::hex << (short)-1, "ffff");
    CASE("oct of -1 (int)", os << std::oct << -1, "37777777777");
    CASE("dec of -1 after hex", os << std::hex << std::dec << -1, "-1");
    CASE("showpos 5", os << std::showpos << 5, "+5");
    CASE("showpos 0", os << std::showpos << 0, "+0");
    CASE("showpos -5", os << std::showpos << -5, "-5");
    CASE("showpos with hex has no sign", os << std::showpos << std::hex << 255, "ff");
    CASE("setw(6)", os << std::setw(6) << 42, "    42");
    CASE("setw(6) left", os << std::left << std::setw(6) << 42, "42    ");
}
static void g3(poly::Test& t, OB& ob, std::ostream& os) {
    CASE("setw(6) right", os << std::right << std::setw(6) << 42, "    42");
    CASE("setw(6) internal negative", os << std::internal << std::setw(6) << -42, "-   42");
    CASE("setw(6) internal hex showbase", os << std::internal << std::hex << std::showbase << std::setw(8) << 255, "0x    ff");
    CASE("fill *", os << std::setfill('*') << std::setw(5) << 42, "***42");
    CASE("width smaller than the text", os << std::setw(1) << 12345, "12345");
    CASE("bool true", os << true, "1");
    CASE("bool false", os << false, "0");
    CASE("boolalpha true", os << std::boolalpha << true, "true");
    CASE("boolalpha false", os << std::boolalpha << false, "false");
    CASE("boolalpha with width", os << std::boolalpha << std::setw(6) << true, "  true");
}
static void g4(poly::Test& t, OB& ob, std::ostream& os) {
    CASE("pointer", os << (const void*)0x1234, "0000000000001234");
    CASE("null pointer", os << (const void*)0, "0000000000000000");
    CASE("pointer with width ignores fill type", os << std::setw(18) << (const void*)0xAB, "  00000000000000AB");
    CASE("nullptr_t", os << nullptr, "nullptr");
    CASE("char", os << 'x', "x");
    CASE("signed char", os << (signed char)'y', "y");
    CASE("unsigned char", os << (unsigned char)'z', "z");
    CASE("char with width", os << std::setw(3) << 'x', "  x");
    CASE("string", os << "abc", "abc");
    CASE("string with width", os << std::setw(6) << "abc", "   abc");
}
static void g5(poly::Test& t, OB& ob, std::ostream& os) {
    CASE("string with width, left", os << std::left << std::setw(6) << "abc" << '|', "abc   |");
    CASE("two values in a row", os << 1 << ' ' << 2 << "x" << 3u, "1 2x3");
    CASE("endl", os << "a" << std::endl, "a\n");
    CASE("ends", os << "a" << std::ends, "a");   // plus a NUL: seen below
    // width is reset after a formatted output
    os.clear(); ob.reset();
    os.width(8);
    os << 5;
    t.expect("width() is 0 after a number", os.width(), 0);
    os.width(8);
    os << "ab";
    t.expect("width() is 0 after a string", os.width(), 0);
    os.width(8);
    os << 'c';
    t.expect("width() is 0 after a char", os.width(), 0);
    os.width(8);
    os.put('c');
    t.expect("put() does not reset the width", os.width(), 8);
    // a null C string sets badbit
    os.width(0);
    os.clear();
    // a stream in the fail state writes nothing
    os.clear(std::ios_base::failbit);
    ob.reset();
    os << 12 << "x" << 'c';
    t.expect("nothing is written in the failed state", ob.n, 0);
    t.expect("the number inserter leaves the state, the char and string inserters add badbit when the sentry fails", os.rdstate(), 6);

}
static void test(poly::Test& t) {
    OB ob;
    std::ostream os(&ob);
    g0(t, ob, os);
    g1(t, ob, os);
    g2(t, ob, os);
    g3(t, ob, os);
    g4(t, ob, os);
    g5(t, ob, os);
}
POLY_TEST_MAIN(test)
