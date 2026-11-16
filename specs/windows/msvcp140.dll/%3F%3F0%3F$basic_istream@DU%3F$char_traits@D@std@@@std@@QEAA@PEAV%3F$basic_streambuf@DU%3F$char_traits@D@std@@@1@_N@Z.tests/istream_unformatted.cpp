// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_istream<char> get() (all forms), getline() (both), ignore(), peek(), read(), readsome(), putback(), unget(), gcount(), sync(), tellg(), seekg() (two forms), operator>>(streambuf*) and _Ipfx(): reads, delimiters, counts and the state bits.
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
struct IBuf : std::streambuf {
    char data[256]; int pos = 0, len = 0;
    poly::Log log;
    int avail = 3;          // showmanyc() result
    int sync_result = 0;
    long long seek_result = 11;
    IBuf(const char* s) { while (s[len]) { data[len] = s[len]; len++; } }
    int underflow() override { return pos < len ? (unsigned char)data[pos] : -1; }
    int uflow() override { return pos < len ? (unsigned char)data[pos++] : -1; }
    int pbackfail(int c) override { log.add_num("pbackfail", c); if (pos > 0) { pos--; return c == -1 ? 0 : c; } return -1; }
    long long showmanyc() override { return avail; }
    int sync() override { log.add("sync"); return sync_result; }
    std::streampos seekoff(long long off, std::ios_base::seekdir dir, std::ios_base::openmode m) override {
        log.add_num("seekoff", off); log.add_num("dir", dir); log.add_num("mode", m); return std::streampos(seek_result);
    }
    std::streampos seekpos(std::streampos p, std::ios_base::openmode m) override {
        log.add_num("seekpos", (long long)p); log.add_num("mode", m); return std::streampos(seek_result);
    }
};
static void g1_b0(poly::Test& t) {
    {
        IBuf sb("abc");
        std::istream is(&sb);
        t.expect("get() returns the character", is.get(), 'a');
        t.expect("gcount is 1", is.gcount(), 1);
        char c = 0;
        is.get(c);
        t.expect("get(char&)", c, 'b');
        t.expect("gcount is 1", is.gcount(), 1);
        t.expect("peek() does not consume", is.peek(), 'c');
        t.expect("peek() resets gcount to 0", is.gcount(), 0);
        t.expect("get() again", is.get(), 'c');
        t.expect("get() at the end returns EOF", is.get(), -1);
        t.expect("eofbit and failbit", is.rdstate(), 3);
        t.expect("gcount is 0 after a failed get", is.gcount(), 0);
        is.clear();
        t.expect("peek() at the end is EOF", is.peek(), -1);
        t.expect("peek at the end sets eofbit only", is.rdstate(), 1);
        is.clear();
        char d = 'z';
        is.get(d);
        t.expect("get(char&) at the end leaves the char", d, 'z');
        t.expect("and sets eofbit|failbit", is.rdstate(), 3);
    }
}
static void g1_b1(poly::Test& t) {
    {
        IBuf sb("hello world\nnext");
        std::istream is(&sb);
        char buf[32];
        memset(buf, '#', sizeof buf);
        is.get(buf, 6);
        t.expect_str("get(buf, 6) reads up to 5 characters and adds a NUL", buf, "hello");
        t.expect("gcount is 5", is.gcount(), 5);
        t.expect("the next character is the blank (not consumed)", is.peek(), ' ');
        is.get(buf, 32);
        t.expect_str("get(buf, n) stops at the newline", buf, " world");
        t.expect("gcount is 6", is.gcount(), 6);
        t.expect("the newline is not extracted", is.peek(), '\n');
        is.get(buf, 32);
        t.expect("get(buf, n) with the newline first extracts nothing", is.gcount(), 0);
        t.expect("and sets failbit", is.rdstate(), 2);
        t.expect_str("the buffer holds just the NUL", buf, "");
        is.clear();
        is.get(buf, 32, 'x');
        t.expect_str("get(buf, n, delim) with a custom delimiter reads through newlines", buf, "\nne");
        is.clear();
        is.get(buf, 32);
        t.expect_str("get() hit the end", buf, "xt");
        t.expect("eofbit is set but not failbit when characters were read", is.rdstate(), 1);
        is.clear();
        is.get(buf, 0);
        t.expect("get(buf, 0) sets failbit", is.rdstate(), 2);
    }
}
static void g1_b2(poly::Test& t) {
    {
        IBuf sb("line1\nline2\n\nlast");
        std::istream is(&sb);
        char buf[32];
        is.getline(buf, 32);
        t.expect_str("getline", buf, "line1");
        t.expect("gcount counts the newline", is.gcount(), 6);
        is.getline(buf, 32);
        t.expect_str("getline again", buf, "line2");
        is.getline(buf, 32);
        t.expect_str("an empty line", buf, "");
        t.expect("gcount is 1", is.gcount(), 1);
        t.expect("no error", is.rdstate(), 0);
        is.getline(buf, 32);
        t.expect_str("the last line without a newline", buf, "last");
        t.expect("eofbit set", is.rdstate(), 1);
        is.clear();
        is.getline(buf, 32);
        t.expect("getline at the end sets eofbit|failbit", is.rdstate(), 3);
        t.expect("gcount 0", is.gcount(), 0);
    }
}
static void g1_b3(poly::Test& t) {
    {
        IBuf sb("abcdefghij");
        std::istream is(&sb);
        char buf[5];
        is.getline(buf, 5);
        t.expect_str("a line longer than the buffer: reads n-1 characters", buf, "abcd");
        t.expect("and sets failbit", is.rdstate(), 2);
        is.clear();
        IBuf sb2("a,b,c");
        std::istream is2(&sb2);
        char two[8];
        is2.getline(two, 8, ',');
        t.expect_str("getline with a delimiter", two, "a");
        is2.getline(two, 8, ',');
        t.expect_str("again", two, "b");
        t.expect("the delimiter was extracted", is2.gcount(), 2);
    }
}
static void g1(poly::Test& t) {
    g1_b0(t);
    g1_b1(t);
    g1_b2(t);
    g1_b3(t);
}
static void g2_b0(poly::Test& t) {
    {
        IBuf sb("0123456789");
        std::istream is(&sb);
        is.ignore();
        t.expect("ignore() skips one", is.peek(), '1');
        is.ignore(3);
        t.expect("gcount is 3", is.gcount(), 3);
        t.expect("ignore(3)", is.peek(), '4');
        is.ignore(100, '8');
        t.expect("gcount 5 (the characters 4..8, the delimiter included)", is.gcount(), 5);
        t.expect("ignore(n, delim) stops after the delimiter", is.peek(), '9');
        is.ignore(100);
        t.expect("ignore to the end sets eofbit", is.rdstate(), 1);
        t.expect("gcount 1", is.gcount(), 1);
    }
}
static void g2_b1(poly::Test& t) {
    {
        IBuf sb("abcdef");
        std::istream is(&sb);
        char buf[8];
        memset(buf, 0, sizeof buf);
        is.read(buf, 4);
        t.expect_str("read(4)", buf, "abcd");
        t.expect("gcount 4", is.gcount(), 4);
        is.read(buf, 4);
        t.expect("a short read: gcount counts what was read", is.gcount(), 2);
        t.expect("and sets eofbit|failbit", is.rdstate(), 3);
        is.clear();
        IBuf sb2("abcdef");
        std::istream is2(&sb2);
        t.expect("readsome returns what showmanyc() says is there, capped by the request", is2.readsome(buf, 10), 3);
        t.expect("readsome(…) 2", is2.readsome(buf, 2), 2);
        sb2.avail = 0;
        t.expect("readsome with nothing available", is2.readsome(buf, 2), 0);
        t.expect("and no state change", is2.rdstate(), 0);
        sb2.avail = -1;
        t.expect("readsome when showmanyc() returns -1 sets eofbit", is2.readsome(buf, 2), 0);
        t.expect("eofbit", is2.rdstate(), 1);
    }
}
static void g2_b2(poly::Test& t) {
    {
        IBuf sb("abc");
        std::istream is(&sb);
        is.get();
        is.get();
        is.unget();
        t.expect("unget() moves back", is.peek(), 'b');
        t.expect("unget() leaves the state alone", is.rdstate(), 0);
        sb.log.clear();
        is.putback('b');
        t.expect("putback() moves back through pbackfail() whatever the character", is.peek(), 'a');
        t.expect_str("putback calls the buffer's pbackfail", sb.log.str(), "pbackfail98");
        sb.log.clear();
        is.get(); is.get(); is.get();
        is.clear();
        is.putback('c');
        t.expect("putback after hitting the end clears eofbit first", is.rdstate(), 0);
        IBuf sb2("x");
        std::istream is2(&sb2);
        is2.unget();
        t.expect("unget() at the start fails with badbit", is2.rdstate(), 4);
        is2.clear();
        is2.putback('q');
        t.expect("putback() at the start fails with badbit", is2.rdstate(), 4);
    }
}
static void g2_b3(poly::Test& t) {
    {
        IBuf sb("abc");
        std::istream is(&sb);
        t.expect("sync() returns the buffer's pubsync()", is.sync(), 0);
        t.expect_str("sync -> sync", sb.log.str(), "sync");
        sb.log.clear();
        sb.sync_result = -1;
        t.expect("a failing sync returns -1", is.sync(), -1);
        t.expect("and sets badbit", is.rdstate(), 4);
        is.clear();
        sb.sync_result = 0;
        sb.log.clear();
        t.expect("tellg returns the buffer's position", (long long)is.tellg(), 11);
        t.expect_str("tellg -> seekoff(0, cur, in)", sb.log.str(), "seekoff0 dir1 mode1");
        sb.log.clear();
        is.seekg(std::streampos(7));
        t.expect_str("seekg(pos) -> seekpos(pos, in)", sb.log.str(), "seekpos7 mode1");
        sb.log.clear();
        is.seekg(-2, std::ios_base::end);
        t.expect_str("seekg(off, dir) -> seekoff(off, dir, in)", sb.log.str(), "seekoff-2 dir2 mode1");
        t.expect("fine so far", is.rdstate(), 0);
        sb.seek_result = -1;
        is.seekg(std::streampos(1));
        t.expect("a failing seekg sets failbit", is.rdstate(), 2);
        t.expect("tellg on a failed stream is -1", (long long)is.tellg(), -1);
        is.clear();
        t.expect("tellg when the buffer fails: -1", (long long)is.tellg(), -1);
        is.clear(std::ios_base::eofbit);
        sb.seek_result = 5;
        is.seekg(std::streampos(1));
        t.expect("seekg clears eofbit first", is.rdstate(), 0);
    }
}
struct Dst : std::streambuf {
    char o[64]; int n = 0;
    int overflow(int c) override { if (c != -1) o[n++] = (char)c; return c == -1 ? 0 : c; }
    const char* str() { o[n] = 0; return o; }
};
static Dst* g_dstp;
static void g2_b4a(poly::Test& t) {
    g_dstp = new Dst();
    Dst& g_dst = *g_dstp;
    IBuf sb("pass it on");
    std::istream is(&sb);
    is >> &g_dst;
    t.expect_str("operator>>(streambuf*) copies everything", g_dst.str(), "pass it on");
    t.expect("and hits the end", is.rdstate(), 1);
}
static void g2_b4b(poly::Test& t) {
    Dst& g_dst = *g_dstp;
    IBuf sb2("abc");
    std::istream is2(&sb2);
    is2.get();
    is2.get(g_dst);
    t.expect_str("get(streambuf&) copies to the end of line/file", g_dst.str(), "pass it onbc");
}
static void g2_b4c(poly::Test& t) {
    Dst& g_dst = *g_dstp;
    IBuf sb3("12,34");
    std::istream is3(&sb3);
    is3.get(g_dst, ',');
    t.expect_str("get(streambuf&, delim) stops before the delimiter", g_dst.str(), "pass it onbc12");
    t.expect("gcount counts the characters copied", is3.gcount(), 2);
}
static void g2_b4(poly::Test& t) {
    g2_b4a(t);
    g2_b4b(t);
    g2_b4c(t);
}
static void g2_b5a(poly::Test& t) {
    IBuf sb("  x");
    std::istream is(&sb);
    t.check("_Ipfx(false) is true on a good stream", is._Ipfx(false), 0);
    t.expect("_Ipfx(false) (the argument is \"noskip\") skipped the leading white space (skipws is set)", sb.pos, 2);
}
static void g2_b5b(poly::Test& t) {
    IBuf sb("  x");
    std::istream is(&sb);
    bool r = is._Ipfx(true);
    t.check("_Ipfx(true) is true on a good stream", r, 0);
    t.expect("_Ipfx(true) (noskip) skips nothing", sb.pos, 0);
    is.unsetf(std::ios_base::skipws);
    is._Ipfx(false);
    t.expect("without skipws _Ipfx(false) skips nothing either", sb.pos, 0);
}
static void g2_b5c(poly::Test& t) {
    IBuf sb("  x");
    std::istream is(&sb);
    is.setstate(std::ios_base::eofbit);
    bool r = is._Ipfx(true);
    t.check("_Ipfx on a stream with only eofbit: false", !r, 0);
    t.expect("and it sets failbit", is.rdstate(), 3);
}
static void g2_b5(poly::Test& t) {
    g2_b5a(t);
    g2_b5b(t);
    g2_b5c(t);
}
static void g2(poly::Test& t) {
    g2_b0(t);
    g2_b1(t);
    g2_b2(t);
    g2_b3(t);
    g2_b4(t);
    g2_b5(t);
}
static void test(poly::Test& t) {
    g1(t);
    g2(t);
}
POLY_TEST_MAIN(test)
