// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::ios_base (??0ios_base@std@@IEAA@XZ, ?_Init@ios_base@std@@IEAAXXZ, ??1ios_base@std@@UEAA@XZ): the object's fields (by offset) after the constructor, after _Init(), and the destructor's effect.
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
struct IB : std::ios_base {
    IB() {}
    void init() { _Init(); }
};
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
// offsets: vptr 0, _Stdstr 8, _Mystate 16, _Except 20, _Fmtfl 24, _Prec 32, _Wide 40, _Arr 48, _Calls 56, _Ploc 64; size 72
static void dump(poly::Test& t, const char* when, void* b) {
    poly::Line l;
    l.str("value ").str(when).str(" stdstr=").dec((poly_i64)F<unsigned long long>(b, 8)).str(" state=").dec(F<int>(b, 16)).str(" except=").dec(F<int>(b, 20))
        .str(" fmtfl=").hex((poly_u32)F<int>(b, 24)).str(" prec=").dec(F<long long>(b, 32)).str(" wide=").dec(F<long long>(b, 40))
        .str(" arr=").dec(F<void*>(b, 48) != 0).str(" calls=").dec(F<void*>(b, 56) != 0).str(" ploc=").dec(F<void*>(b, 64) != 0);
    l.end();
}
static void test(poly::Test& t) {
    t.value("sizeof(ios_base)", sizeof(std::ios_base));
    t.value("alignof(ios_base)", alignof(std::ios_base));
    t.value("sizeof(IB)", sizeof(IB));
    IB* b = (IB*)malloc(sizeof(IB));
    memset(b, 0xCC, sizeof(IB));
    new (b) IB();
    dump(t, "after-ctor", b);
    t.check("the constructor alone leaves the locale pointer null", F<void*>(b, 64) == 0, 0);
    t.expect("and _Stdstr 0, state 0", (poly_i64)F<unsigned long long>(b, 8) + F<int>(b, 16), 0);
    t.check("the object starts with a vtable pointer", *(void**)b != 0, 0);
    b->init();
    dump(t, "after-init", b);
    t.expect("state is goodbit", F<int>(b, 16), 0);
    t.expect("exceptions mask is 0", F<int>(b, 20), 0);
    t.expect_hex("format flags are skipws|dec", (poly_u32)F<int>(b, 24), 0x201);
    t.expect("precision is 6", F<long long>(b, 32), 6);
    t.expect("width is 0", F<long long>(b, 40), 0);
    t.expect("_Stdstr is 0", (poly_i64)F<unsigned long long>(b, 8), 0);
    t.check("no iword/pword array, no callbacks", F<void*>(b, 48) == 0 && F<void*>(b, 56) == 0, 0);
    t.check("a locale object was allocated", F<void*>(b, 64) != 0, 0);
    t.check("it holds the global locale", b->getloc() == std::locale(), 0);
    t.check("it is the classic locale", b->getloc() == std::locale::classic(), 0);
    t.expect_str("its name is C", b->getloc().name().c_str(), "C");
    t.check("good() is true", b->good(), 0);
    t.check("eof() is false", !b->eof(), 0);
    t.check("fail() is false", !b->fail(), 0);
    t.check("bad() is false", !b->bad(), 0);
    t.check("operator bool is true", (bool)*b, 0);
    t.check("operator! is false", !*b == false, 0);
    // a second _Init resets the fields
    b->flags(0x4000);
    b->width(7);
    b->precision(2);
    b->setstate(std::ios_base::eofbit);
    b->init();
    t.expect_hex("flags back to skipws|dec after another _Init", (poly_u32)b->flags(), 0x201);
    t.expect("precision 6 again", b->precision(), 6);
    t.expect("width 0 again", b->width(), 0);
    t.expect("state cleared again", b->rdstate(), 0);
    b->~IB();
    t.check("the destructor ran", true, 0);
}
POLY_TEST_MAIN(test)
