// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::ios_base flags(), setf(), unsetf(), width(), precision(), exceptions(): the stored values and what each call returns.
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
    IB() { _Init(); }
};
static void test(poly::Test& t) {
    IB* b = new IB();
    t.expect_hex("initial flags", (poly_u32)b->flags(), 0x201);
    int old = b->flags(0x0FFF);
    t.expect_hex("flags(x) returns the old flags", (poly_u32)old, 0x201);
    t.expect_hex("flags() now", (poly_u32)b->flags(), 0x0FFF);
    old = b->flags(0xFFFFFFFF);
    t.expect_hex("flags(all ones) returns the old flags", (poly_u32)old, 0x0FFF);
    t.expect_hex("only the low 16 bits are kept", (poly_u32)b->flags(), 0xFFFF);
    b->flags(0);
    old = b->setf(std::ios_base::showpos | std::ios_base::uppercase);
    t.expect_hex("setf(f) returns the old flags", (poly_u32)old, 0);
    t.expect_hex("setf(f) merged", (poly_u32)b->flags(), 0x24);
    old = b->setf(std::ios_base::hex, std::ios_base::basefield);
    t.expect_hex("setf(f, mask) returns the old flags", (poly_u32)old, 0x24);
    t.expect_hex("setf(f, mask) replaced the masked bits", (poly_u32)b->flags(), 0x824);
    b->setf(std::ios_base::oct, std::ios_base::basefield);
    t.expect_hex("a second masked setf clears the other bits of the mask", (poly_u32)b->flags(), 0x424);
    b->setf(0x1FFFF);
    t.expect_hex("setf ignores bits above 16", (poly_u32)b->flags(), 0xFFFF);
    b->unsetf(std::ios_base::showbase | std::ios_base::left);
    t.expect_hex("unsetf clears bits", (poly_u32)b->flags(), 0xFFFF & ~0x48);
    b->flags(0);
    b->setf(0xFFFF, 0x00F0);
    t.expect_hex("setf(f, mask) only changes bits in mask", (poly_u32)b->flags(), 0x00F0);
    // width and precision
    t.expect("width() is 0", b->width(), 0);
    t.expect("width(10) returns 0", b->width(10), 0);
    t.expect("width() is 10", b->width(), 10);
    t.expect("width(-3) returns 10", b->width(-3), 10);
    t.expect("negative widths are stored", b->width(), -3);
    t.expect("precision() is 6", b->precision(), 6);
    t.expect("precision(0) returns 6", b->precision(0), 6);
    t.expect("precision() is 0", b->precision(), 0);
    t.expect("precision(1<<40) returns 0", b->precision(1LL << 40), 0);
    t.expect("64-bit precision is stored", b->precision(), 1LL << 40);
    b->width(1LL << 33);
    t.expect("64-bit width is stored", b->width(), 1LL << 33);
    // exceptions mask is masked with 0x17
    b->exceptions(0xFF);
    t.expect_hex("exceptions() keeps the low status bits", (poly_u32)b->exceptions(), 0x17);
    b->exceptions(0);
    t.expect("exceptions(0)", b->exceptions(), 0);
    // getloc/imbue
    std::locale l0 = b->getloc();
    t.check("getloc returns a copy of the global locale", l0 == std::locale(), 0);
    std::locale prev = b->imbue(std::locale::classic());
    t.check("imbue returns the previous locale", prev == l0, 0);
    delete b;
}
POLY_TEST_MAIN(test)
