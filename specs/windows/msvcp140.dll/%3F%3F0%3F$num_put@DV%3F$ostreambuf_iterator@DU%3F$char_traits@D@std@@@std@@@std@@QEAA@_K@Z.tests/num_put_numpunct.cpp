// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::num_put<char, ostreambuf_iterator<char>> (?put@, ?do_put@, ?_Fput@, ?_Iput@, ?_Ffmt@, ?_Ifmt@, ?_Put@, ?_Rep@, ??0?$num_put@...QEAA@_K@Z, ?id@?$num_put@...) and std::numpunct<char> (?id@?$numpunct@D@std@@2V0locale@2@A, the layout of the program's own facet object): thousands grouping, decimal point and bool names taken from a facet in the stream's locale, and direct use of the facet.
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
    char b[512]; int n = 0;
    int overflow(int c) override { if (c != -1) b[n++] = (char)c; return c == -1 ? 0 : c; }
    long long xsputn(const char* s, long long k) override { for (long long i = 0; i < k; i++) b[n++] = s[i]; return k; }
    const char* str() { b[n] = 0; return b; }
    void reset() { n = 0; }
};
struct Punct : std::numpunct<char> {
    char dp, ts; const char* grp; const char* tn; const char* fn;
    Punct(char d, char t, const char* g, const char* y, const char* no) : std::numpunct<char>((size_t)0), dp(d), ts(t), grp(g), tn(y), fn(no) {}
    char do_decimal_point() const override { return dp; }
    char do_thousands_sep() const override { return ts; }
    std::string do_grouping() const override { return grp; }
    std::string do_truename() const override { return tn; }
    std::string do_falsename() const override { return fn; }
};
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
#define CASE(name, expr, want) do { ob.reset(); os.clear(); os.flags(0x201); os.width(0); os.precision(6); os.fill(' '); { expr; } t.expect_str(name, ob.str(), want); } while (0)
static void g1(poly::Test& t) {
    // the classic numpunct facet, as the program makes it
    const std::numpunct<char>& np = std::use_facet<std::numpunct<char>>(std::locale::classic());
    t.value("sizeof(numpunct<char>)", sizeof(std::numpunct<char>));
    t.expect("decimal_point() is '.'", np.decimal_point(), '.');
    t.expect("thousands_sep() is ','", np.thousands_sep(), ',');
    t.expect_str("grouping() is empty", np.grouping().c_str(), "");
    t.expect_str("truename()", np.truename().c_str(), "true");
    t.expect_str("falsename()", np.falsename().c_str(), "false");
    void* o = (void*)&np;
    t.check("a vtable pointer", *(void**)o != 0, 0);
    t.expect("the object: _Dp at +24 is '.'", F<char>(o, 24), '.');
    t.expect("the object: _Kseparator at +25 is ','", F<char>(o, 25), ',');
    t.check("the object: _Grouping at +16 is an empty C string", F<const char*>(o, 16) != 0 && F<const char*>(o, 16)[0] == 0, 0);
    t.expect_str("the object: _Falsename at +32", F<const char*>(o, 32), "false");
    t.expect_str("the object: _Truename at +40", F<const char*>(o, 40), "true");
    t.value("numpunct<char>::id", (poly_i64)F<unsigned long long>(&std::numpunct<char>::id, 0));
}
static void g2(poly::Test& t) {
    OB ob;
    std::ostream os(&ob);
    std::locale loc(std::locale::classic(), new Punct('.', ',', "\3", "yes", "no"));
    os.imbue(loc);
    CASE("grouping of 3: 1234567", os << 1234567, "1,234,567");
    CASE("grouping with a negative number", os << -1234567, "-1,234,567");
    CASE("no grouping for a short number", os << 123, "123");
    CASE("999 and 1000", os << 999 << ' ' << 1000, "999 1,000");
    CASE("hex numbers are grouped too", os << std::hex << 0x123456, "123,456");
    CASE("unsigned long long", os << 18446744073709551615ULL, "18,446,744,073,709,551,615");
    CASE("double with grouping in the integer part", os << std::fixed << 1234567.5, "1,234,567.500000");
    CASE("double in default format", os << 1234.5, "1,234.5");
    CASE("boolalpha true uses truename()", os << std::boolalpha << true, "yes");
    CASE("boolalpha false uses falsename()", os << std::boolalpha << false, "no");
    CASE("width counts the grouping characters", os << std::setw(12) << 1234567, "   1,234,567");
    CASE("pointers are grouped too (the 16 hex digits)", os << (const void*)0x12345678, "0,000,000,012,345,678");
    os.imbue(std::locale(std::locale::classic(), new Punct(',', '.', "\2\3", "ja", "nein")));
    CASE("a decimal comma", os << 1.5, "1,5");
    CASE("grouping 2 then 3 (the first group is the rightmost; the last size repeats)", os << 123456789, "1.234.567.89");
    CASE("fixed with a decimal comma and the grouping 2,3", os << std::fixed << std::setprecision(2) << 1234.567, "12.34,57");
    CASE("scientific with a decimal comma", os << std::scientific << 1.5, "1,500000e+00");
    CASE("boolalpha uses the new names", os << std::boolalpha << true, "ja");
    os.imbue(std::locale(std::locale::classic(), new Punct('.', '\'', "\x7f", "t", "f")));
    CASE("a group size of CHAR_MAX means no grouping", os << 12345678, "12345678");
    os.imbue(std::locale(std::locale::classic(), new Punct('.', '_', "\1", "t", "f")));
    CASE("a group size of 1", os << 12345, "1_2_3_4_5");
    os.imbue(std::locale(std::locale::classic(), new Punct('.', ',', "", "t", "f")));
    CASE("an empty grouping string means no grouping", os << 12345678, "12345678");
}
static void g3(poly::Test& t) {
    // direct use of the facet through ostreambuf_iterator
    OB ob;
    std::ostream os(&ob);
    const std::num_put<char>& np = std::use_facet<std::num_put<char>>(std::locale::classic());
    t.value("sizeof(num_put<char>)", sizeof(std::num_put<char>));
    std::ostreambuf_iterator<char> it(&ob);
    std::ostreambuf_iterator<char> r = np.put(it, os, '*', 42L);
    t.check("put returns an iterator that has not failed", !r.failed(), 0);
    t.expect_str("put(long)", ob.str(), "42");
    ob.reset();
    os.width(6);
    np.put(it, os, '*', -7L);
    t.expect_str("put with width and fill", ob.str(), "****-7");
    t.expect("put resets the stream's width", os.width(), 0);
    ob.reset();
    np.put(it, os, ' ', 255UL);
    t.expect_str("put(unsigned long)", ob.str(), "255");
    ob.reset();
    np.put(it, os, ' ', 9223372036854775807LL);
    t.expect_str("put(long long)", ob.str(), "9223372036854775807");
    ob.reset();
    np.put(it, os, ' ', 18446744073709551615ULL);
    t.expect_str("put(unsigned long long)", ob.str(), "18446744073709551615");
    ob.reset();
    np.put(it, os, ' ', 2.5);
    t.expect_str("put(double)", ob.str(), "2.5");
    ob.reset();
    np.put(it, os, ' ', (long double)0.125);
    t.expect_str("put(long double)", ob.str(), "0.125");
    ob.reset();
    np.put(it, os, ' ', true);
    t.expect_str("put(bool)", ob.str(), "1");
    ob.reset();
    os.setf(std::ios_base::boolalpha);
    np.put(it, os, ' ', false);
    t.expect_str("put(bool) with boolalpha", ob.str(), "false");
    os.unsetf(std::ios_base::boolalpha);
    ob.reset();
    np.put(it, os, ' ', (const void*)0xBEEF);
    t.expect_str("put(const void*)", ob.str(), "000000000000BEEF");
    ob.reset();
    os.setf(std::ios_base::hex, std::ios_base::basefield);
    os.setf(std::ios_base::showbase | std::ios_base::uppercase);
    np.put(it, os, ' ', 255L);
    t.expect_str("the format comes from the stream's flags", ob.str(), "0XFF");
    // a failing buffer: the returned iterator reports failure
    struct Fail : std::streambuf {
        int overflow(int c) override { return -1; }
    } fb;
    std::ostreambuf_iterator<char> fit(&fb);
    std::ostreambuf_iterator<char> fr = np.put(fit, os, ' ', 12345L);
    t.check("put into a buffer that refuses characters: failed()", fr.failed(), 0);
}
static void test(poly::Test& t) {
    g1(t);
    g2(t);
    g3(t);
}
POLY_TEST_MAIN(test)
