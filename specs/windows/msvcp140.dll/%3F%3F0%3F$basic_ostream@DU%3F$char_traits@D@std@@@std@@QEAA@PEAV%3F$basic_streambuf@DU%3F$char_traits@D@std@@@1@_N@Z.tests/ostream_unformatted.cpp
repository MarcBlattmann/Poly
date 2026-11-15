// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_ostream<char> put(), write(), flush(), tellp(), seekp() (two forms), operator<<(streambuf*) and the manipulators: how each reaches the stream buffer.
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
    poly::Log log;
    char b[512]; int n = 0;
    int sync_result = 0;
    long long pos_result = 40;
    int overflow(int c) override { if (c != -1) b[n++] = (char)c; return c == -1 ? 0 : c; }
    long long xsputn(const char* s, long long k) override {
        log.add_num("xsputn", k);
        for (long long i = 0; i < k; i++) b[n++] = s[i];
        return k;
    }
    int sync() override { log.add("sync"); return sync_result; }
    std::streampos seekoff(long long off, std::ios_base::seekdir dir, std::ios_base::openmode m) override {
        log.add_num("seekoff", off); log.add_num("dir", dir); log.add_num("mode", m);
        return std::streampos(pos_result);
    }
    std::streampos seekpos(std::streampos p, std::ios_base::openmode m) override {
        log.add_num("seekpos", (long long)p); log.add_num("mode", m);
        return std::streampos(pos_result);
    }
    const char* str() { b[n] = 0; return b; }
};
struct SrcBuf : std::streambuf {
    const char* s; int i = 0;
    SrcBuf(const char* x) : s(x) {}
    int underflow() override { return s[i] ? (unsigned char)s[i] : -1; }
    int uflow() override { return s[i] ? (unsigned char)s[i++] : -1; }
};
static void test(poly::Test& t) {
    LB lb;
    std::ostream os(&lb);
    os.put('a');
    t.expect_str("put writes one character", lb.str(), "a");
    t.check("put returns the stream", &os.put('b') == &os, 0);
    t.expect_str("put again", lb.str(), "ab");
    os.write("cdef", 4);
    t.expect_str("write puts the bytes", lb.str(), "abcdef");
    t.expect_str("write calls xsputn(4)", lb.log.str(), "xsputn4");
    lb.log.clear();
    os.write("x\0y", 3);
    t.expect("write can write NUL bytes", lb.n, 9);
    t.expect_str("one xsputn(3)", lb.log.str(), "xsputn3");
    lb.log.clear();
    os.write("", 0);
    t.text("the xsputn log of write(\"\", 0)", lb.log.str());
    lb.log.clear();
    // flush
    os.flush();
    t.expect_str("flush calls rdbuf()->pubsync() once", lb.log.str(), "sync");
    lb.log.clear();
    t.check("flush returns the stream", &os.flush() == &os, 0);
    lb.log.clear();
    lb.sync_result = -1;
    os.flush();
    t.expect("a sync that returns -1 sets badbit", os.rdstate(), 4);
    lb.sync_result = 0;
    os.clear();
    lb.log.clear();
    os.setstate(std::ios_base::failbit);
    os.flush();
    t.expect_str("flush on a failed stream does not call sync", lb.log.str(), "");
    os.clear();
    // endl, ends, flush manipulators
    lb.n = 0;
    os << "t" << std::endl;
    t.expect_str("endl writes a newline", lb.str(), "t\n");
    t.expect_str("and flushes", lb.log.str(), "xsputn1 sync");
    lb.log.clear();
    os << std::flush;
    t.expect_str("the flush manipulator flushes", lb.log.str(), "sync");
    lb.log.clear();
    // tellp / seekp
    t.expect("tellp returns the buffer's position", (long long)os.tellp(), 40);
    t.expect_str("tellp -> seekoff(0, cur, out)", lb.log.str(), "seekoff0 dir1 mode2");
    lb.log.clear();
    os.seekp(std::streampos(12));
    t.expect_str("seekp(pos) -> seekpos(pos, out)", lb.log.str(), "seekpos12 mode2");
    lb.log.clear();
    os.seekp(-3, std::ios_base::end);
    t.expect_str("seekp(off, dir) -> seekoff(off, dir, out)", lb.log.str(), "seekoff-3 dir2 mode2");
    t.expect("good after successful seeks", os.rdstate(), 0);
    lb.log.clear();
    lb.pos_result = -1;
    os.seekp(std::streampos(5));
    t.expect("seekp failing sets failbit", os.rdstate(), 2);
    os.clear();
    os.seekp(1, std::ios_base::cur);
    t.expect("seekp(off, dir) failing sets failbit", os.rdstate(), 2);
    t.expect("tellp on a failed stream is -1", (long long)os.tellp(), -1);
    lb.log.clear();
    os.clear();
    t.expect("tellp when the buffer fails returns -1", (long long)os.tellp(), -1);
    os.clear();
    lb.pos_result = 40;
    // << streambuf*
    {
        SrcBuf src("copy me");
        lb.n = 0;
        lb.log.clear();
        os << &src;
        t.expect_str("operator<<(streambuf*) copies everything", lb.str(), "copy me");
        t.expect("no error bits", os.rdstate(), 0);
        SrcBuf empty("");
        os << &empty;
        t.expect("an empty source sets failbit", os.rdstate(), 2);
        os.clear();
        os << (std::streambuf*)0;
        t.expect("a null streambuf* sets badbit", os.rdstate() & 4, 4);
        os.clear();
    }
    // the stream buffer is locked and unlocked around formatted output (no observable effect here)
    t.expect("put() on a good stream", os.good(), 1);
}
POLY_TEST_MAIN(test)
