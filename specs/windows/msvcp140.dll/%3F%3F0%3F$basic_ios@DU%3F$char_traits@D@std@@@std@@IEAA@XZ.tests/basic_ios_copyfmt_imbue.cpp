// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_ios<char> copyfmt(), imbue(), swap(), move(), set_rdbuf(): which parts are copied or exchanged, and the call into the buffer's imbue().
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
struct LB : std::streambuf {
    int imbued = 0;
    void imbue(const std::locale& l) override { imbued++; }
};
struct BI : std::ios {
    BI(std::streambuf* sb) { init(sb); }
};
struct MyFacet : std::locale::facet {
    static std::locale::id id;
    MyFacet() : facet(0) {}
};
std::locale::id MyFacet::id;
static poly::Log g_log;
static void cb(std::ios_base::event ev, std::ios_base&, int idx) {
    g_log.add_num("e", ev);
    g_log.add_num("i", idx);
}
static void test(poly::Test& t) {
    LB lb1, lb2;
    BI* a = new BI(&lb1);
    BI* b = new BI(&lb2);
    std::ostream tie_a(&lb1);
    a->fill('a');
    a->tie(&tie_a);
    a->width(5);
    a->precision(3);
    a->flags(0x4001);
    a->setstate(std::ios_base::failbit);
    a->register_callback(cb, 4);
    // copyfmt
    g_log.clear();
    b->copyfmt(*a);
    t.expect("copyfmt copies the fill", b->fill(), 'a');
    t.check("copyfmt copies the tie", b->tie() == &tie_a, 0);
    t.expect("copyfmt copies the width", b->width(), 5);
    t.expect("copyfmt copies the precision", b->precision(), 3);
    t.expect_hex("copyfmt copies the flags", (poly_u32)b->flags(), 0x4001);
    t.expect("copyfmt does not copy the state", b->rdstate(), 0);
    t.check("copyfmt does not copy the buffer", b->rdbuf() == &lb2, 0);
    t.expect_str("copyfmt called the copied callback with copyfmt_event", g_log.str(), "e2 i4");
    b->copyfmt(*b);
    t.check("copyfmt onto itself is a no-op", b->fill() == 'a', 0);
    // exceptions mask is copied, and may throw
    a->exceptions(std::ios_base::eofbit);
    b->clear(std::ios_base::eofbit);
    bool thrown = false;
    try { b->copyfmt(*a); } catch (const std::ios_base::failure&) { thrown = true; }
    t.check("copyfmt throws when the copied exception mask matches the target's state", thrown, 0);
    t.expect("the mask was copied", b->exceptions(), 1);
    a->exceptions(0);
    b->exceptions(0);
    b->clear();
    // imbue
    std::locale loc(std::locale::classic(), new MyFacet);
    t.check("the new locale differs from classic", !(loc == std::locale::classic()), 0);
    g_log.clear();
    std::locale prev = a->imbue(loc);
    t.check("imbue returns the previous locale", prev == std::locale::classic(), 0);
    t.check("getloc() is the new one", a->getloc() == loc, 0);
    t.expect("the stream buffer's imbue() was called once", lb1.imbued, 1);
    t.check("the buffer's own locale is the new one", lb1.getloc() == loc, 0);
    t.expect_str("the imbue_event callback ran", g_log.str(), "e1 i4");
    t.expect("the other buffer is untouched", lb2.imbued, 0);
    // swap exchanges everything but rdbuf
    b->fill('b');
    b->width(1);
    b->tie(0);
    a->swap(*b);
    t.expect("swap: fill", a->fill(), 'b');
    t.expect("swap: fill (2)", b->fill(), 'a');
    t.expect("swap: width", a->width(), 1);
    t.check("swap: tie", a->tie() == 0 && b->tie() == &tie_a, 0);
    t.check("swap: buffers stay", a->rdbuf() == &lb1 && b->rdbuf() == &lb2, 0);
    t.expect("swap: state is exchanged", a->rdstate(), 0);
    t.expect("swap: state is exchanged (2)", b->rdstate(), 2);
    t.check("swap: the locale pointers are exchanged", b->getloc() == loc && a->getloc() == std::locale::classic(), 0);
    // move: swap, with this's buffer and tie cleared first
    a->move(*b);
    t.check("move: the target keeps no buffer", a->rdbuf() == 0, 0);
    t.check("move: the target took the tie", a->tie() == &tie_a, 0);
    t.expect("move: the target took the fill", a->fill(), 'a');
    t.check("move: the source keeps its buffer", b->rdbuf() == &lb2, 0);
    // set_rdbuf changes only the pointer
    a->clear();
    a->set_rdbuf(&lb1);
    t.check("set_rdbuf installs the buffer", a->rdbuf() == &lb1, 0);
    t.expect("set_rdbuf leaves the state as it is (badbit from clear() with no buffer)", a->rdstate(), 4);
}
POLY_TEST_MAIN(test)
