// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_streambuf<char> get and put areas (?setg@, ?setp@ (two forms), ?gbump@, ?pbump@, ?eback@, ?gptr@, ?egptr@, ?pbase@, ?pptr@, ?epptr@, ?_Gnavail@, ?_Pnavail@, ?_Gninc@, ?_Gndec@, ?_Gnpreinc@, ?_Pninc@, ?xsgetn@, ?xsputn@, ?uflow@): the pointer arithmetic and the default xsgetn/xsputn/uflow working on them.
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
struct RB : std::streambuf {
    const char* src = "0123456789";
    int pos = 0;
    char chunk[4];
    int fills = 0;
    int underflow() override {
        if (gptr() < egptr()) return (unsigned char)*gptr();
        if (pos >= 10) return -1;
        int n = 10 - pos < 3 ? 10 - pos : 3;
        for (int i = 0; i < n; i++) chunk[i] = src[pos + i];
        pos += n;
        fills++;
        setg(chunk, chunk, chunk + n);
        return (unsigned char)*gptr();
    }
    char* Eback() { return eback(); }
    char* Gptr() { return gptr(); }
    char* Egptr() { return egptr(); }
    void Gbump(int n) { gbump(n); }
    long long Gnavail() { return _Gnavail(); }
    char* Gninc() { return _Gninc(); }
    char* Gndec() { return _Gndec(); }
    char* Gnpreinc() { return _Gnpreinc(); }
    void Setg(char* a, char* b, char* c) { setg(a, b, c); }
};
struct WB : std::streambuf {
    char area[4];
    char out[64];
    int n = 0;
    int overflows = 0;
    WB() { setp(area, area + 4); }
    void drain() { for (char* p = pbase(); p < pptr(); p++) out[n++] = *p; setp(area, area + 4); }
    int overflow(int c) override {
        overflows++;
        drain();
        if (c != -1) { *pptr() = (char)c; pbump(1); }
        return c == -1 ? 0 : c;
    }
    char* Pbase() { return pbase(); }
    char* Pptr() { return pptr(); }
    char* Epptr() { return epptr(); }
    void Pbump(int k) { pbump(k); }
    long long Pnavail() { return _Pnavail(); }
    char* Pninc() { return _Pninc(); }
    void Setp(char* a, char* b) { setp(a, b); }
    void Setp3(char* a, char* b, char* c) { setp(a, b, c); }
};
static void test(poly::Test& t) {
    // get area arithmetic
    RB r;
    char a[10] = {'0', '1', '2', '3', '4', '5', '6', '7', '8', '9'};
    r.Setg(a, a + 2, a + 8);
    t.check("setg(first, next, last): eback, gptr, egptr", r.Eback() == a && r.Gptr() == a + 2 && r.Egptr() == a + 8, 0);
    t.expect("_Gnavail is last - next", r.Gnavail(), 6);
    r.Gbump(3);
    t.check("gbump moves gptr and shrinks the count", r.Gptr() == a + 5 && r.Egptr() == a + 8 && r.Gnavail() == 3, 0);
    r.Gbump(-2);
    t.check("a negative gbump moves back", r.Gptr() == a + 3 && r.Gnavail() == 5, 0);
    t.check("_Gninc returns the old position and advances", r.Gninc() == a + 3 && r.Gptr() == a + 4, 0);
    t.check("_Gnpreinc advances and returns the new position", r.Gnpreinc() == a + 5 && r.Gptr() == a + 5 && r.Gnavail() == 3, 0);
    t.check("_Gndec steps back and returns the new position", r.Gndec() == a + 4 && r.Gptr() == a + 4 && r.Gnavail() == 4, 0);
    r.Setg(0, 0, 0);
    t.expect("_Gnavail is 0 when gptr is null", r.Gnavail(), 0);
    // default xsgetn / uflow refill through underflow()
    RB s;
    char buf[16];
    memset(buf, 0, sizeof buf);
    t.expect("sgetn(7) through the default xsgetn", s.sgetn(buf, 7), 7);
    t.expect_str("the characters in order", buf, "0123456");
    t.expect("three refills of up to 3 characters", s.fills, 3);
    t.expect("the next character is 7", s.sgetc(), '7');
    t.expect("sgetn(10) stops at the end of the data", s.sgetn(buf, 10), 3);
    t.expect("end: sgetc is EOF", s.sgetc(), -1);
    t.expect("end: sbumpc is EOF", s.sbumpc(), -1);
    t.expect("end: snextc is EOF", s.snextc(), -1);
    RB u;
    t.expect("sbumpc through the default uflow", u.sbumpc(), '0');
    t.expect("in_avail is what is left in the area", u.in_avail(), 2);
    t.expect("snextc", u.snextc(), '2');
    t.expect("sbumpc", u.sbumpc(), '2');
    t.expect("sbumpc refills", u.sbumpc(), '3');
    t.expect("in_avail after a refill", u.in_avail(), 2);
    // put area arithmetic
    WB w;
    t.check("setp(first, last): pbase = pptr = first, epptr = last", w.Pbase() == w.area && w.Pptr() == w.area && w.Epptr() == w.area + 4, 0);
    t.expect("_Pnavail is the room left", w.Pnavail(), 4);
    t.expect("sputc into the area", w.sputc('A'), 'A');
    t.check("pptr advanced", w.Pptr() == w.area + 1 && w.Pnavail() == 3, 0);
    w.Pbump(2);
    t.check("pbump(2) moves pptr and shrinks the count", w.Pptr() == w.area + 3 && w.Pnavail() == 1, 0);
    w.Pbump(-1);
    t.check("a negative pbump moves back", w.Pptr() == w.area + 2 && w.Pnavail() == 2, 0);
    t.check("_Pninc returns the old position and advances", w.Pninc() == w.area + 2 && w.Pptr() == w.area + 3, 0);
    w.Setp3(w.area, w.area + 2, w.area + 4);
    t.check("setp(first, next, last): pptr = next", w.Pbase() == w.area && w.Pptr() == w.area + 2 && w.Epptr() == w.area + 4 && w.Pnavail() == 2, 0);
    w.Setp(0, 0);
    t.expect("_Pnavail is 0 when pptr is null", w.Pnavail(), 0);
    w.Setp(w.area, w.area + 4);
    // default xsputn: fills the area, then overflow() for each further character
    WB x;
    t.expect("sputn(10) through the default xsputn", x.sputn("0123456789", 10), 10);
    x.drain();
    x.out[x.n] = 0;
    t.expect_str("every character arrived, in order", x.out, "0123456789");
    t.expect("overflow() was called for the characters that did not fit", x.overflows, 2);
    WB y;
    t.expect("sputn(0)", y.sputn("abc", 0), 0);
    t.expect("no overflow", y.overflows, 0);
    t.expect("sputn(3) fits", y.sputn("abc", 3), 3);
    t.expect("still no overflow", y.overflows, 0);
    t.expect("sputn(1) exactly fills the area", y.sputn("d", 1), 1);
    t.expect("and still no overflow", y.overflows, 0);
    t.expect("one more character goes through overflow", y.sputc('e'), 'e');
    t.expect("overflow count 1", y.overflows, 1);
    // _Init with the six pointers redirects the areas
    struct Redir : std::streambuf {
        char* g1 = 0; char* g2 = 0; int gc = 0; char* p1 = 0; char* p2 = 0; int pc = 0;
        char ext[8];
        Redir() {
            g1 = ext; g2 = ext; gc = 4;
            p1 = ext + 4; p2 = ext + 4; pc = 4;
            _Init(&g1, &g2, &gc, &p1, &p2, &pc);
        }
        char* Gptr() { return gptr(); }
        char* Pptr() { return pptr(); }
        char* Egptr() { return egptr(); }
        char* Epptr() { return epptr(); }
    };
    Redir d;
    d.ext[0] = 'k'; d.ext[1] = 'l';
    t.check("_Init(six pointers) makes the buffer use the caller's storage", d.Gptr() == d.ext && d.Egptr() == d.ext + 4 && d.Pptr() == d.ext + 4 && d.Epptr() == d.ext + 8, 0);
    t.expect("and reads through it", d.sbumpc(), 'k');
    t.check("the caller's pointer moved", d.g2 == d.ext + 1 && d.gc == 3, 0);
    t.expect("and writes through it", d.sputc('Z'), 'Z');
    t.check("the caller's put pointer moved", d.p2 == d.ext + 5 && d.pc == 3 && d.ext[4] == 'Z', 0);
}
POLY_TEST_MAIN(test)
