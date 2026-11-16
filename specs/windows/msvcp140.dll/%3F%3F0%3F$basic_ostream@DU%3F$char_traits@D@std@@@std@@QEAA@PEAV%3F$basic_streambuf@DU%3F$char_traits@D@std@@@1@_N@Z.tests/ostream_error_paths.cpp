// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_ostream<char> (?_Osfx@, ?opfx@, ?osfx@, ??6?$basic_ostream@... and the sentry the inserters use): tied streams, unitbuf, output failure, an exception thrown by the stream buffer, and the exception mask.
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
    int syncs = 0; int sync_result = 0;
    int fail_after = 1 << 30;     // overflow fails once this many characters were written
    int throw_after = 1 << 30;    // overflow/xsputn throw once this many characters were written
    int overflow(int c) override {
        if (n >= throw_after) throw 77;
        if (n >= fail_after) return -1;
        if (c != -1) b[n++] = (char)c;
        return c == -1 ? 0 : c;
    }
    long long xsputn(const char* s, long long k) override {
        for (long long i = 0; i < k; i++) { if (overflow((unsigned char)s[i]) == -1) return i; }
        return k;
    }
    int sync() override { syncs++; return sync_result; }
    const char* str() { b[n] = 0; return b; }
};
static void g0(poly::Test& t) {
    bool thrown = false; std::string what; int code = 0; int s0 = 0; int before = 0; (void)thrown; (void)what; (void)code; (void)s0; (void)before;

    // tie: the tied stream is flushed before output
    OB a, b;
    std::ostream tied(&a), os(&b);
    os.tie(&tied);
    os << 'x';
    t.expect("the tied stream was flushed (sync called) before the output", a.syncs, 1);
    os << "yz";
    t.expect("and again for the next insertion", a.syncs, 2);
    t.expect_str("the output went to the other buffer", b.str(), "xyz");
    os.put('w');
    t.expect("put() has a sentry too: it flushes the tie", a.syncs, 3);
    os.write("v", 1);
    t.expect("write() as well", a.syncs, 4);
    os.tie(&os);
    os << 'q';
    t.expect("a stream tied to itself does not flush itself", b.syncs, 0);
    os.tie(0);
    // a tied stream in the fail state stops the output
    os.tie(&tied);
    tied.setstate(std::ios_base::badbit);
    before = b.n;
    os << 'r';
    t.expect("a bad tied stream does not stop the output (only this stream's own state counts after the flush)", b.n, before + 1);
    tied.clear();
    os.tie(0);

}
static void g1(poly::Test& t) {
    bool thrown = false; std::string what; int code = 0; int s0 = 0; int before = 0; (void)thrown; (void)what; (void)code; (void)s0; (void)before;
    // unitbuf: _Osfx syncs after each insertion
    OB u;
    std::ostream us(&u);
    us.setf(std::ios_base::unitbuf);
    us << 1;
    t.expect("unitbuf: one sync after an insertion", u.syncs, 1);
    us << "ab" << 'c';
    t.expect("unitbuf: one sync per insertion (\"ab\" and 'c' are two)", u.syncs, 3);
    us.put('x');
    us.write("yz", 2);
    t.expect("put() and write() have their own sentry: sync each", u.syncs, 5);
    u.sync_result = -1;
    us << 2;
    t.expect("a failing sync sets badbit", us.rdstate() & 4, 4);
    us.clear();
    u.sync_result = 0;
    us.unsetf(std::ios_base::unitbuf);
    s0 = u.syncs;
    us << 3;
    t.expect("without unitbuf: no sync", u.syncs, s0);
    // calling _Osfx / opfx / osfx directly
    us.setf(std::ios_base::unitbuf);
    s0 = u.syncs;
    us.osfx();
    t.expect("osfx() syncs when unitbuf is set", u.syncs, s0 + 1);
    us._Osfx();
    t.expect("_Osfx() syncs when unitbuf is set", u.syncs, s0 + 2);
    us.unsetf(std::ios_base::unitbuf);
    us._Osfx();
    t.expect("_Osfx() does nothing without unitbuf", u.syncs, s0 + 2);
    t.check("opfx() is true on a good stream", us.opfx(), 0);
    us.setstate(std::ios_base::failbit);
    t.check("opfx() is false on a failed stream", !us.opfx(), 0);
    us.clear();

}
static void g2(poly::Test& t) {
    bool thrown = false; std::string what; int code = 0; int s0 = 0; int before = 0; (void)thrown; (void)what; (void)code; (void)s0; (void)before;
    // output failure: the buffer refuses a character
    OB f;
    f.fail_after = 3;
    std::ostream fs(&f);
    fs << "abcdef";
    t.expect_str("the buffer took three characters", f.str(), "abc");
    t.expect("a failed put sets badbit", fs.rdstate(), 4);
    fs << "more";
    t.expect_str("further output is skipped in the bad state", f.str(), "abc");
    fs.clear();
    f.n = 0;
    f.fail_after = 0;
    fs << 12345;
    t.expect("a number whose first character is refused sets badbit", fs.rdstate(), 4);
    fs.clear();
    fs.put('x');
    t.expect("put() on a refusing buffer sets badbit", fs.rdstate(), 4);
    fs.clear();
    fs.write("abc", 3);
    t.expect("write() that stores fewer than asked sets badbit", fs.rdstate(), 4);
    // with the mask set, the failure becomes an exception
    fs.clear();
    fs.exceptions(std::ios_base::badbit);
    thrown = false;
    try { fs << 99; } catch (const std::ios_base::failure& e) { thrown = true; what = e.what(); }
    t.check("badbit in the mask: an ios_base::failure is thrown", thrown, 0);
    t.expect_str("its text", what.c_str(), "ios_base::badbit set: iostream stream error");
    t.expect("the state has badbit", fs.rdstate(), 4);

}
static void g3(poly::Test& t) {
    bool thrown = false; std::string what; int code = 0; int s0 = 0; int before = 0; (void)thrown; (void)what; (void)code; (void)s0; (void)before;
    // an exception thrown by the stream buffer
    OB x;
    x.throw_after = 2;
    std::ostream xs(&x);
    thrown = false;
    try { xs << "abcd"; } catch (...) { thrown = true; }
    t.check("without badbit in the mask the buffer's exception is swallowed", !thrown, 0);
    t.expect("and badbit is set", xs.rdstate(), 4);
    t.expect_str("the first characters were written", x.str(), "ab");
    xs.clear();
    xs.exceptions(std::ios_base::badbit);
    code = 0;
    try { xs << "efgh"; } catch (int v) { code = v; } catch (...) { code = -1; }
    t.expect("with badbit in the mask the original exception is rethrown unchanged", code, 77);
    t.expect("and badbit is set", xs.rdstate(), 4);
    xs.exceptions(0);
    xs.clear();
    code = 0;
    try { xs.put('k'); } catch (...) { code = -1; }
    t.expect("put() swallows the buffer's exception too when the mask is empty", code, 0);
    t.expect("and sets badbit", xs.rdstate(), 4);
    xs.clear();
    xs.exceptions(std::ios_base::failbit);
    code = 0;
    try { xs.put('k'); } catch (...) { code = -1; }
    t.expect("an exception not matching the mask is swallowed", code, 0);

}
static void g4(poly::Test& t) {
    bool thrown = false; std::string what; int code = 0; int s0 = 0; int before = 0; (void)thrown; (void)what; (void)code; (void)s0; (void)before;
    // a stream with no buffer
    std::ostream nb(0);
    nb << 5 << "x";
    t.expect("writing to a stream without a buffer leaves badbit", nb.rdstate() & 4, 4);
    nb.flush();
    t.expect("flush on it is harmless", nb.rdstate() & 4, 4);

}
static void test(poly::Test& t) {
    g0(t);
    g1(t);
    g2(t);
    g3(t);
    g4(t);
}
POLY_TEST_MAIN(test)
