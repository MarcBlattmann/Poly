// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::basic_istream<char> and basic_iostream<char> (??0?$basic_istream@...QEAA@PEAV?$basic_streambuf@...@1@_N@Z, ??0?$basic_iostream@..., ??_D?$basic_istream@..., ??_D?$basic_iostream@..., ??1?$basic_istream@..., ??1?$basic_iostream@...): the hidden virtual-base argument, the object layout, and the _Chcount member.
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
    char data[64]; int pos = 0, len = 0;
    char out[64]; int on = 0;
    IBuf(const char* s) { while (s[len]) { data[len] = s[len]; len++; } }
    int underflow() override { return pos < len ? (unsigned char)data[pos] : -1; }
    int uflow() override { return pos < len ? (unsigned char)data[pos++] : -1; }
    int overflow(int c) override { if (c != -1) out[on++] = (char)c; return c == -1 ? 0 : c; }
    const char* ostr() { out[on] = 0; return out; }
};
struct MyIS : std::istream {
    int extra = 456;
    MyIS(std::streambuf* sb) : std::istream(sb) {}
    ~MyIS() { destroyed++; }
    static int destroyed;
};
int MyIS::destroyed = 0;
struct MyIOS : std::iostream {
    int extra = 789;
    MyIOS(std::streambuf* sb) : std::iostream(sb) {}
    ~MyIOS() { destroyed++; }
    static int destroyed;
};
int MyIOS::destroyed = 0;
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
static void test(poly::Test& t) {
    t.value("sizeof(basic_istream<char>)", sizeof(std::istream));
    t.value("sizeof(basic_iostream<char>)", sizeof(std::iostream));
    t.value("sizeof(MyIS)", sizeof(MyIS));
    t.value("sizeof(MyIOS)", sizeof(MyIOS));
    IBuf sb("abc");
    {
        std::istream is(&sb);
        void* o = &is;
        int* vbt = F<int*>(o, 0);
        t.check("a virtual-base table pointer at offset 0", vbt != 0, 0);
        t.expect("vbtable[0] is 0", vbt[0], 0);
        t.expect("vbtable[1] is the offset of the basic_ios base", vbt[1], (char*)(std::ios*)&is - (char*)&is);
        t.value("offset of the basic_ios base in istream", (char*)(std::ios*)&is - (char*)&is);
        t.value("istream: the 8 bytes at +8 (the character count _Chcount)", F<long long>(o, 8));
        t.expect("gcount() is 0", is.gcount(), 0);
        t.check("rdbuf()", is.rdbuf() == &sb && is.good(), 0);
        t.expect("fill is a blank", is.fill(), ' ');
        t.expect_hex("flags skipws|dec", (poly_u32)is.flags(), 0x201);
        t.check("no tie", is.tie() == 0, 0);
        F<long long>(o, 8) = 77;
        t.expect("gcount() reads the member at +8", is.gcount(), 77);
        t.expect("get() clears it and counts one", (is.get(), is.gcount()), 1);
    }
    {
        // the second boolean argument of the istream constructor is the "do not initialise the virtual base" flag the compiler passes, not part of the source's interface;
        // the source-level second constructor is (streambuf*, bool isstd, bool initbase)
        std::istream st(&sb, true);
        t.check("isstd = true", st.good() && st.rdbuf() == &sb, 0);
    }
    {
        std::istream bad(0);
        t.expect("a stream constructed with a null buffer has badbit", bad.rdstate(), 4);
    }
    {
        IBuf io("xy");
        std::iostream ios(&io);
        void* o = &ios;
        t.value("iostream: offset of the basic_ios base", (char*)(std::ios*)&ios - (char*)&ios);
        t.value("iostream: offset of the istream sub-object", (char*)(std::istream*)&ios - (char*)&ios);
        t.value("iostream: offset of the ostream sub-object", (char*)(std::ostream*)&ios - (char*)&ios);
        t.value("iostream: the 8 bytes at +0", F<long long>(o, 0) != 0);
        t.value("iostream: the 8 bytes at +8 (_Chcount of the istream part)", F<long long>(o, 8));
        t.value("iostream: the vbptr of the ostream part at +16 is set", F<long long>(o, 16) != 0);
        t.value("iostream: vbtable of the istream part [0]", F<int*>(o, 0)[0]);
        t.value("iostream: vbtable of the istream part [1]", F<int*>(o, 0)[1]);
        t.value("iostream: vbtable of the ostream part [0]", F<int*>(o, 16)[0]);
        t.value("iostream: vbtable of the ostream part [1]", F<int*>(o, 16)[1]);
        t.expect("get() reads from the buffer", ios.get(), 'x');
        ios.put('Q');
        t.expect_str("put() writes to the same buffer", io.ostr(), "Q");
        t.expect("gcount is shared with the istream part", ios.gcount(), 1);
        t.check("both parts share one basic_ios (rdbuf)", ((std::istream&)ios).rdbuf() == ((std::ostream&)ios).rdbuf(), 0);
    }
    // a user class derived from istream / iostream
    {
        IBuf b2("pq");
        MyIS m(&b2);
        t.expect("the derived member is intact", m.extra, 456);
        t.expect("it reads", m.get(), 'p');
        t.value("offset of the ios base in MyIS", (char*)(std::ios*)&m - (char*)&m);
        t.value("offset of extra in MyIS", (char*)&m.extra - (char*)&m);
    }
    t.expect("the derived destructor ran once", MyIS::destroyed, 1);
    {
        IBuf b3("rs");
        MyIOS m(&b3);
        t.expect("the iostream's derived member is intact", m.extra, 789);
        t.expect("it reads", m.get(), 'r');
        t.value("offset of the ios base in MyIOS", (char*)(std::ios*)&m - (char*)&m);
        t.value("offset of extra in MyIOS", (char*)&m.extra - (char*)&m);
    }
    t.expect("the derived iostream destructor ran once", MyIOS::destroyed, 1);
    // deletion through the virtual-base pointer
    IBuf b4("t");
    MyIS* mp = new MyIS(&b4);
    std::ios_base* ip = (std::istream*)mp;
    delete ip;
    t.expect("delete through an ios_base* ran the derived destructor", MyIS::destroyed, 2);
    MyIOS* mq = new MyIOS(&b4);
    delete (std::ios_base*)(std::ios*)mq;
    t.expect("the same through the iostream's ios_base*", MyIOS::destroyed, 2);
    MyIOS* mr = new MyIOS(&b4);
    delete (std::istream*)mr;
    t.expect("delete through an istream* of an iostream", MyIOS::destroyed, 3);
    MyIOS* ms = new MyIOS(&b4);
    delete (std::ostream*)ms;
    t.expect("delete through an ostream* of an iostream", MyIOS::destroyed, 4);
}
POLY_TEST_MAIN(test)
