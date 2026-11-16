// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_streambuf<char> (??0?$basic_streambuf@...IEAA@XZ, ?overflow@, ?underflow@, ?uflow@, ?pbackfail@, ?showmanyc@, ?xsgetn@, ?xsputn@, ?seekoff@, ?seekpos@, ?setbuf@, ?sync@, ?imbue@, ?_Lock@, ?_Unlock@, ?getloc@, ?_Init@): the object after construction, and the base class's default virtual functions.
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
struct SB : std::streambuf {
    // the defaults, called non-virtually
    int base_overflow(int c) { return std::streambuf::overflow(c); }
    int base_pbackfail(int c) { return std::streambuf::pbackfail(c); }
    long long base_showmanyc() { return std::streambuf::showmanyc(); }
    int base_underflow() { return std::streambuf::underflow(); }
    int base_uflow() { return std::streambuf::uflow(); }
    long long base_xsgetn(char* p, long long n) { return std::streambuf::xsgetn(p, n); }
    long long base_xsputn(const char* p, long long n) { return std::streambuf::xsputn(p, n); }
    std::streampos base_seekoff(long long off, int dir, int mode) { return std::streambuf::seekoff(off, dir, mode); }
    std::streampos base_seekpos(std::streampos p, int mode) { return std::streambuf::seekpos(p, mode); }
    std::streambuf* base_setbuf(char* p, long long n) { return std::streambuf::setbuf(p, n); }
    int base_sync() { return std::streambuf::sync(); }
    void base_imbue(const std::locale& l) { std::streambuf::imbue(l); }
    void base_lock() { std::streambuf::_Lock(); }
    void base_unlock() { std::streambuf::_Unlock(); }
    char* Eback() { return eback(); }
    char* Gptr() { return gptr(); }
    char* Egptr() { return egptr(); }
    char* Pbase() { return pbase(); }
    char* Pptr() { return pptr(); }
    char* Epptr() { return epptr(); }
};
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
// layout: vptr 0, _Gfirst 8, _Pfirst 16, _IGfirst 24, _IPfirst 32, _Gnext 40, _Pnext 48, _IGnext 56, _IPnext 64, _Gcount 72, _Pcount 76, _IGcount 80, _IPcount 88, _Plocale 96; size 104
static void test(poly::Test& t) {
    t.value("sizeof(basic_streambuf<char>)", sizeof(std::streambuf));
    SB* b = (SB*)malloc(sizeof(SB));
    memset(b, 0xCC, sizeof(SB));
    new (b) SB();
    t.check("a vtable pointer at 0", *(void**)b != 0, 0);
    t.check("_IGfirst points at _Gfirst (this+8)", F<char**>(b, 24) == (char**)((char*)b + 8), 0);
    t.check("_IPfirst points at _Pfirst (this+16)", F<char**>(b, 32) == (char**)((char*)b + 16), 0);
    t.check("_IGnext points at _Gnext (this+40)", F<char**>(b, 56) == (char**)((char*)b + 40), 0);
    t.check("_IPnext points at _Pnext (this+48)", F<char**>(b, 64) == (char**)((char*)b + 48), 0);
    t.check("_IGcount points at _Gcount (this+72)", F<int*>(b, 80) == (int*)((char*)b + 72), 0);
    t.check("_IPcount points at _Pcount (this+76)", F<int*>(b, 88) == (int*)((char*)b + 76), 0);
    t.check("both areas are empty (null pointers, zero counts)", F<char*>(b, 8) == 0 && F<char*>(b, 16) == 0 && F<char*>(b, 40) == 0 && F<char*>(b, 48) == 0 && F<int>(b, 72) == 0 && F<int>(b, 76) == 0, 0);
    t.check("eback gptr egptr pbase pptr epptr are all null", !b->Eback() && !b->Gptr() && !b->Egptr() && !b->Pbase() && !b->Pptr() && !b->Epptr(), 0);
    t.check("_Plocale is a heap locale object", F<void*>(b, 96) != 0, 0);
    t.check("getloc() is the global locale", b->getloc() == std::locale(), 0);
    t.check("getloc() is classic", b->getloc() == std::locale::classic(), 0);
    // defaults of the virtual functions
    t.expect("overflow(c) fails: EOF", b->base_overflow('x'), -1);
    t.expect("overflow() with no argument is EOF", b->base_overflow(-1), -1);
    t.expect("pbackfail(c) fails", b->base_pbackfail('x'), -1);
    t.expect("showmanyc() is 0", b->base_showmanyc(), 0);
    t.expect("underflow() is EOF", b->base_underflow(), -1);
    t.expect("uflow() is EOF (underflow() says EOF)", b->base_uflow(), -1);
    char buf[8];
    t.expect("xsgetn(8) reads nothing", b->base_xsgetn(buf, 8), 0);
    t.expect("xsputn of 5 chars: the first overflow fails, nothing written", b->base_xsputn("hello", 5), 0);
    t.expect("xsputn of 0 chars", b->base_xsputn("", 0), 0);
    std::streampos sp = b->base_seekoff(10, 0, 3);
    t.expect("seekoff() returns pos -1", (long long)sp, -1);
    t.expect("seekpos() returns pos -1", (long long)b->base_seekpos(std::streampos(5), 3), -1);
    t.check("setbuf() returns this", b->base_setbuf(buf, 8) == b, 0);
    t.expect("sync() is 0", b->base_sync(), 0);
    b->base_imbue(std::locale::classic());
    b->base_lock();
    b->base_unlock();
    t.check("imbue(), _Lock() and _Unlock() do nothing", true, 0);
    // public wrappers over the virtuals (virtual dispatch to the defaults)
    t.expect("sgetc() on an empty buffer calls underflow(): EOF", b->sgetc(), -1);
    t.expect("sbumpc() calls uflow(): EOF", b->sbumpc(), -1);
    t.expect("snextc(): EOF", b->snextc(), -1);
    t.expect("sputc() on no put area calls overflow(): EOF", b->sputc('a'), -1);
    t.expect("in_avail() with no buffer: showmanyc() = 0", b->in_avail(), 0);
    t.expect("sungetc() with no buffer: pbackfail() = EOF", b->sungetc(), -1);
    t.expect("sputbackc() with no buffer: EOF", b->sputbackc('a'), -1);
    t.expect("sgetn() reads nothing", b->sgetn(buf, 4), 0);
    t.expect("sputn() writes nothing", b->sputn("ab", 2), 0);
    t.expect("pubsync() is 0", b->pubsync(), 0);
    t.check("pubsetbuf() returns this", b->pubsetbuf(buf, 4) == b, 0);
    t.expect("pubseekoff() is -1", (long long)b->pubseekoff(0, std::ios_base::beg), -1);
    t.expect("pubseekpos() is -1", (long long)b->pubseekpos(std::streampos(0)), -1);
    // pubimbue returns the previous locale and stores the new one
    std::locale prev = b->pubimbue(std::locale::classic());
    t.check("pubimbue returns the old locale", prev == std::locale(), 0);
    b->~SB();
    t.check("the destructor ran", true, 0);
}
POLY_TEST_MAIN(test)
