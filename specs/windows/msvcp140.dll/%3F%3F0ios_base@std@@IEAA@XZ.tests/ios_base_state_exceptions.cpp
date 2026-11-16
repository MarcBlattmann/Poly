// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::ios_base clear(), setstate(), rdstate(), exceptions(): state masking, and the ios_base::failure thrown for a state bit in the exception mask (what() texts, error code, category).
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
// error_category::_Addr (the identity of a standard category object)
static unsigned long long addr_of(const std::error_category& c) { return *(const unsigned long long*)((const char*)&c + 8); }
static void test(poly::Test& t) {
    IB* b = new IB();
    b->clear(0xFF);
    t.expect_hex("clear(0xFF) keeps eofbit|failbit|badbit and bit 4 (0x17)", (poly_u32)b->rdstate(), 0x17);
    b->clear();
    t.expect("clear() resets", b->rdstate(), 0);
    b->setstate(std::ios_base::eofbit);
    t.check("eof() after setstate(eofbit)", b->eof() && !b->fail() && !b->bad() && !b->good(), 0);
    t.check("operator bool still true with only eofbit", (bool)*b, 0);
    b->setstate(std::ios_base::failbit);
    t.expect("eofbit and failbit", b->rdstate(), 3);
    t.check("fail() true, operator bool false", b->fail() && !(bool)*b && !*b, 0);
    b->clear(std::ios_base::badbit);
    t.check("clear(badbit) replaces the state", b->rdstate() == 4 && b->bad() && b->fail(), 0);
    b->clear();
    // exceptions
    struct Case { int bit; const char* what; } cases[] = {
        {std::ios_base::failbit, "ios_base::failbit set"},
        {std::ios_base::badbit, "ios_base::badbit set"},
        {std::ios_base::eofbit, "ios_base::eofbit set"},
    };
    for (int i = 0; i < 3; i++) {
        b->clear();
        b->exceptions(cases[i].bit);
        bool thrown = false;
        const char* w = "";
        std::string text;
        int val = -1;
        bool is_iostream = false;
        bool is_system_error = false;
        try {
            b->setstate(cases[i].bit);
        } catch (const std::ios_base::failure& e) {
            thrown = true;
            val = e.code().value();
            is_iostream = addr_of(e.code().category()) == 5;
            text = e.what();
            w = text.c_str();
            is_system_error = true;
        }
        t.check("the bit in the mask throws ios_base::failure", thrown, 0);
        t.expect("its error code value is 1 (io_errc::stream)", val, 1);
        t.check("its category is the iostream category (identity 5)", is_iostream, 0);
        poly::Line l;
        l.str("value what ").str(cases[i].what).str(" => ").str(w);
        l.end();
        t.expect_str("what() starts with the message", w, (std::string(cases[i].what) + ": iostream stream error").c_str());
        t.expect_hex("the state was stored before the throw", (poly_u32)b->rdstate(), (poly_u32)cases[i].bit);
    }
    // priority of the messages when several bits are in the mask
    b->clear();
    b->exceptions(7);
    std::string w1, w2, w3;
    try { b->setstate(7); } catch (const std::exception& e) { w1 = e.what(); }
    t.expect_str("badbit wins over failbit and eofbit", w1.c_str(), "ios_base::badbit set: iostream stream error");
    b->clear();
    try { b->clear(3); } catch (const std::exception& e) { w2 = e.what(); }
    t.expect_str("failbit wins over eofbit", w2.c_str(), "ios_base::failbit set: iostream stream error");
    // a state bit outside the mask does not throw
    b->clear();
    b->exceptions(std::ios_base::badbit);
    bool thrown = false;
    try { b->setstate(std::ios_base::failbit); } catch (...) { thrown = true; }
    t.check("failbit not in the mask: no exception", !thrown, 0);
    t.expect("failbit is stored", b->rdstate(), 2);
    // exceptions(mask) re-checks the current state
    b->clear(std::ios_base::failbit);
    thrown = false;
    try { b->exceptions(std::ios_base::failbit); } catch (const std::ios_base::failure&) { thrown = true; }
    t.check("setting the mask while the state matches it throws", thrown, 0);
    t.expect("the mask is stored even though it threw", b->exceptions(), 2);
    // clear(state, reraise): rethrows the exception being handled, instead of throwing a new one
    b->clear();
    b->exceptions(std::ios_base::badbit);
    int caught = 0;
    try {
        try { throw 42; } catch (...) { b->clear(std::ios_base::badbit, true); }
    } catch (int v) { caught = v; } catch (...) { caught = -1; }
    t.expect("clear(state, true) rethrows the current exception", caught, 42);
    b->clear();
    delete b;
}
POLY_TEST_MAIN(test)
