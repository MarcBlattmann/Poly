// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::num_get<char, istreambuf_iterator<char>> (?get@, ?do_get@ (eleven overloads), ?_Getifld@, ?_Getffld@, ?id@, ??_7?$num_get@..., ??0?$num_get@...QEAA@_K@Z): direct use of the facet, the end iterator, the state bits it sets, and its order of virtual functions.
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
    char data[128]; int pos = 0, len = 0;
    IBuf(const char* s) { while (s[len]) { data[len] = s[len]; len++; } }
    int underflow() override { return pos < len ? (unsigned char)data[pos] : -1; }
    int uflow() override { return pos < len ? (unsigned char)data[pos++] : -1; }
};
typedef std::istreambuf_iterator<char> It;
struct MyGet : std::num_get<char> {
    poly::Log* log;
    MyGet(poly::Log* l) : std::num_get<char>(0), log(l) {}
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, bool& v) const override { log->add("bool"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, long& v) const override { log->add("long"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, unsigned short& v) const override { log->add("ushort"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, unsigned int& v) const override { log->add("uint"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, unsigned long& v) const override { log->add("ulong"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, long long& v) const override { log->add("llong"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, unsigned long long& v) const override { log->add("ullong"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, float& v) const override { log->add("float"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, double& v) const override { log->add("double"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, long double& v) const override { log->add("ldouble"); return i; }
    It do_get(It i, It e, std::ios_base&, std::ios_base::iostate&, void*& v) const override { log->add("ptr"); return i; }
};
struct OB : std::streambuf { int overflow(int c) override { return c; } };
static void g1(poly::Test& t) {
    const std::num_get<char>& ng = std::use_facet<std::num_get<char>>(std::locale::classic());
    t.value("sizeof(num_get<char>)", sizeof(std::num_get<char>));
    OB ob;
    std::ostream os(&ob);
    {
        IBuf sb("123 456");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        long v = 0;
        It r = ng.get(b, e, os, st, v);
        t.expect("get(long)", v, 123);
        t.expect("no state bits", st, 0);
        t.expect("the returned iterator points at the blank", *r, ' ');
    }
    {
        IBuf sb("789");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        int dummy = 0;
        unsigned long long v = 0;
        It r = ng.get(b, e, os, st, v);
        t.expect("get(unsigned long long)", (long long)v, 789);
        t.expect("hitting the end sets eofbit", st, 1);
        t.check("the returned iterator is the end", r == e, 0);
    }
    {
        IBuf sb("x1");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        long v = 55;
        ng.get(b, e, os, st, v);
        t.expect("a non-number sets failbit", st, 2);
        t.expect("and stores 0", v, 0);
    }
    {
        IBuf sb("-2.5e2 ");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        double v = 0;
        ng.get(b, e, os, st, v);
        t.expect("get(double)", (long long)v, -250);
        t.expect("state", st, 0);
    }
    {
        IBuf sb("1 ");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        bool v = false;
        ng.get(b, e, os, st, v);
        t.expect("get(bool)", v, 1);
    }
    {
        IBuf sb("false ");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        bool v = true;
        os.setf(std::ios_base::boolalpha);
        ng.get(b, e, os, st, v);
        os.unsetf(std::ios_base::boolalpha);
        t.expect("get(bool) with boolalpha reads the name", v, 0);
        t.expect("state", st, 0);
    }
    {
        IBuf sb("0xff ");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        unsigned v = 0;
        os.setf(std::ios_base::hex, std::ios_base::basefield);
        ng.get(b, e, os, st, v);
        os.setf(std::ios_base::dec, std::ios_base::basefield);
        t.expect("get(unsigned) in hex", v, 255);
    }
    {
        IBuf sb("0000000000001234 ");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        void* v = 0;
        ng.get(b, e, os, st, v);
        t.expect("get(void*)", (long long)v, 0x1234);
    }
    {
        // grouping is read through the stream's numpunct facet: classic has none, so a comma ends the number
        IBuf sb("1,234");
        It b(&sb), e;
        std::ios_base::iostate st = 0;
        long v = 0;
        It r = ng.get(b, e, os, st, v);
        t.expect("the value stops at the comma in the classic locale", v, 1);
        t.expect("the iterator points at the comma", *r, ',');
    }
}
static void g2(poly::Test& t) {
    poly::Log log;
    MyGet* mg = new MyGet(&log);
    OB ob;
    std::ostream os(&ob);
    IBuf sb("");
    It b(&sb), e;
    std::ios_base::iostate st = 0;
    bool vb; long vl; unsigned short vus; unsigned int vu; unsigned long vul; long long vll; unsigned long long vull; float vf; double vd; long double vld; void* vp;
    mg->get(b, e, os, st, vb);
    mg->get(b, e, os, st, vl);
    mg->get(b, e, os, st, vus);
    mg->get(b, e, os, st, vu);
    mg->get(b, e, os, st, vul);
    mg->get(b, e, os, st, vll);
    mg->get(b, e, os, st, vull);
    mg->get(b, e, os, st, vf);
    mg->get(b, e, os, st, vd);
    mg->get(b, e, os, st, vld);
    mg->get(b, e, os, st, vp);
    t.expect_str("get() calls the do_get() overload of its type, in the header's order", log.str(), "bool long ushort uint ulong llong ullong float double ldouble ptr");
    t.expect("_Myrefs is 0", *(long*)((char*)mg + 8), 0);
}
static void test(poly::Test& t) {
    g1(t);
    g2(t);
}
POLY_TEST_MAIN(test)
