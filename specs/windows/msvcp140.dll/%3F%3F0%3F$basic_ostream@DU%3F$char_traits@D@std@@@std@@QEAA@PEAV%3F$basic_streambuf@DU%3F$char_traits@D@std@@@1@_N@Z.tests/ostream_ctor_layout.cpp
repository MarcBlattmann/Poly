// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_ostream<char> (??0?$basic_ostream@...QEAA@PEAV?$basic_streambuf@...@1@_N@Z, ??_D?$basic_ostream@...QEAAXXZ, ??1?$basic_ostream@...UEAA@XZ): the hidden "construct the virtual base" argument, the object layout with its virtual base, and destruction through the base pointers.
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
    char b[256]; int n = 0;
    int overflow(int c) override { if (c != -1) b[n++] = (char)c; return c == -1 ? 0 : c; }
    const char* str() { b[n] = 0; return b; }
};
struct MyOS : std::ostream {
    int extra = 123;
    MyOS(std::streambuf* sb) : std::ostream(sb) {}
    ~MyOS() { destroyed++; }
    static int destroyed;
};
int MyOS::destroyed = 0;
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
static void test(poly::Test& t) {
    t.value("sizeof(basic_ostream<char>)", sizeof(std::ostream));
    t.value("sizeof(MyOS)", sizeof(MyOS));
    OB ob;
    {
        std::ostream os(&ob);
        void* o = &os;
        int* vbt = F<int*>(o, 0);
        t.check("a virtual-base table pointer at offset 0", vbt != 0, 0);
        t.expect("vbtable[0] is 0 (distance of the table pointer from the object)", vbt[0], 0);
        t.expect("vbtable[1] is 16 (offset of the basic_ios base)", vbt[1], 16);
        t.expect("the basic_ios sub-object is at +16", (char*)(std::ios*)&os - (char*)&os, 16);
        t.expect("the basic_ios base starts with its own vtable pointer", F<void*>(o, 16) != 0, 1);
        t.value("the 4 bytes at +12 (vtordisp of the virtual base)", F<int>(o, 12));
        t.value("the 4 bytes at +8", F<int>(o, 8));
        t.check("rdbuf() is the buffer", os.rdbuf() == &ob, 0);
        t.check("good()", os.good(), 0);
        t.expect("fill is a blank", os.fill(), ' ');
        t.expect_hex("flags skipws|dec", (poly_u32)os.flags(), 0x201);
        t.check("no tie", os.tie() == 0, 0);
        t.check("the buffer's fields: nothing written yet", ob.n == 0, 0);
        os.put('a');
        t.expect_str("a character is written through the buffer", ob.str(), "a");
    }
    // the destructor ran (nothing observable but no crash); a stream without a buffer
    {
        std::ostream bad(0);
        t.expect("a stream constructed with a null buffer has badbit", bad.rdstate(), 4);
        t.check("rdbuf() is null", bad.rdbuf() == 0, 0);
    }
    // the second argument: isstd
    {
        std::ostream st(&ob, true);
        t.check("isstd = true: still works", st.good() && st.rdbuf() == &ob, 0);
        t.check("isstd = true: _Stdstr (offset 24) is non-zero", F<unsigned long long>((std::ios*)&st, 8) != 0, 0);
    }
    // a user class derived from ostream: the derived constructor builds the virtual base, then calls the ostream constructor with initialization of the base switched off
    ob.n = 0;
    {
        MyOS m(&ob);
        t.expect("the derived object's own member is intact", m.extra, 123);
        t.check("it is usable as an ostream", m.good() && m.rdbuf() == &ob, 0);
        m.put('b');
        t.expect_str("writes work", ob.str(), "b");
        std::ostream& base = m;
        t.expect("the derived class's own vbtable gives the same offset as the pointer conversion", F<int*>(&m, 0)[1], (char*)(std::ios*)&m - (char*)&m);
        t.value("offset of the ios base in MyOS", (char*)(std::ios*)&m - (char*)&m);
        base.put('c');
        t.expect_str("writes through the base reference", ob.str(), "bc");
        t.value("offset of extra in MyOS", (char*)&m.extra - (char*)&m);
    }
    t.expect("the derived destructor ran once", MyOS::destroyed, 1);
    // destruction through a base pointer: ostream's virtual destructor is reached through the virtual base (the ios_base vtable)
    MyOS* mp = new MyOS(&ob);
    std::ostream* op = mp;
    std::ios_base* ip = op;
    t.expect("pointer conversion to the virtual base adds the vbtable offset", (char*)ip - (char*)mp, F<int*>(mp, 0)[1]);
    delete ip;
    t.expect("delete through an ios_base* ran the derived destructor", MyOS::destroyed, 2);
    MyOS* mp2 = new MyOS(&ob);
    delete (std::ostream*)mp2;
    t.expect("delete through an ostream* ran the derived destructor", MyOS::destroyed, 3);
    // an object that was constructed as most-derived and is destroyed: the buffer is not touched
    t.expect("the buffer content is unchanged by destruction", ob.n, 2);
}
POLY_TEST_MAIN(test)
