// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::locale (?classic@locale@std@@SAAEBV12@XZ, ?global@locale@std@@SA?AV12@AEBV12@@Z, ?_Init@locale@std@@CAPEAV_Locimp@12@_N@Z, ?_Getgloballocale@locale@std@@CAPEAV_Locimp@12@XZ, ?_Setgloballocale@..., ?empty@..., ?_New_Locimp@..., ?_Locimp_ctor@..., ?_Locimp_dtor@..., ?_Locimp_Addfac@..., ?_Clocptr@_Locimp@locale@std@@0PEAV123@EA, ?_Id_cnt@id@locale@std@@0HA): the classic and global locales, copies and reference counts, locales built with an added facet, and a bad locale name.
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
extern "C" __declspec(dllimport) void* __stdcall GetProcAddress(void* module, const char* name);
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
// locale: 16 bytes, _Ptr at 8. _Locimp: vptr 0, _Myrefs (long) 8, _Facetvec 16, _Facetcount 24, _Catmask 32, _Xparent 36, _Name (_Yarn<char>: char* at 40, char at 48) 40; size 56
static void* impl(const std::locale& l) { return F<void*>((void*)&l, 8); }
static long refs(const std::locale& l) { return F<long>(impl(l), 8); }
struct MyFacet : std::locale::facet {
    static std::locale::id id;
    MyFacet() : facet(0) {}
    int value() const { return 7; }
};
std::locale::id MyFacet::id;
struct Other : std::locale::facet {
    static std::locale::id id;
    Other() : facet(0) {}
};
std::locale::id Other::id;
static void test(poly::Test& t) {
    t.value("sizeof(locale)", sizeof(std::locale));
    t.value("sizeof(locale::facet)", sizeof(std::locale::facet));
    t.value("sizeof(locale::id)", sizeof(std::locale::id));
    const std::locale& c1 = std::locale::classic();
    const std::locale& c2 = std::locale::classic();
    t.check("classic() returns the same object each time", &c1 == &c2, 0);
    t.expect_str("the classic locale's name", c1.name().c_str(), "C");
    t.check("the object is a pointer to the implementation at offset 8", impl(c1) != 0, 0);
    t.check("the global locale starts out as the classic one (same implementation)", impl(std::locale()) == impl(c1), 0);
    t.check("the _Clocptr static is the classic implementation", *(void**)GetProcAddress(GetModuleHandleW(L"msvcp140.dll"), "?_Clocptr@_Locimp@locale@std@@0PEAV123@EA") == impl(c1), 0);
    t.expect("the implementation's name pointer is a C string at +40", (long long)(F<char*>(impl(c1), 40)[0]), 'C');
    t.expect("_Xparent of classic is false", F<char>(impl(c1), 36), 0);
    t.expect("_Catmask of classic is all categories (0x3F)", F<int>(impl(c1), 32), 0x3F);
    t.value("_Facetcount of the classic locale", (poly_i64)F<unsigned long long>(impl(c1), 24));
    t.value("_Facetvec of the classic locale is null (its facets are created on first use)", F<void*>(impl(c1), 16) == 0);
    // reference counting
    long r0 = refs(c1);
    {
        std::locale a;
        t.expect("a copy of the global locale adds a reference", refs(c1), r0 + 1);
        std::locale b(a);
        t.expect("another copy adds another", refs(c1), r0 + 2);
        t.check("copies share the implementation", impl(a) == impl(b) && impl(a) == impl(c1), 0);
        b = c1;
        t.expect("assignment of the same implementation changes nothing", refs(c1), r0 + 2);
    }
    t.expect("destruction removes the references", refs(c1), r0);
    // locale("C"): a new implementation with the same name; equality is by name
    {
        std::locale named("C");
        t.expect_str("name of locale(\"C\")", named.name().c_str(), "C");
        t.check("locale(\"C\") == classic()", named == c1, 0);
        t.check("operator!= is the negation", !(named != c1), 0);
        t.value("locale(\"C\") shares classic's implementation", impl(named) == impl(c1));
    }
    // a locale made from another with one facet added: name "*", never equal to another locale
    {
        std::locale x(c1, new MyFacet);
        t.expect_str("name of a locale with a replaced facet is *", x.name().c_str(), "*");
        t.check("it is not equal to classic()", !(x == c1), 0);
        t.check("it is equal to itself (same implementation)", x == x, 0);
        std::locale y(x);
        t.check("and to its copy", x == y, 0);
        t.check("has_facet finds the added facet", std::has_facet<MyFacet>(x), 0);
        t.check("the classic locale does not have it", !std::has_facet<MyFacet>(c1), 0);
        t.expect("use_facet returns it", std::use_facet<MyFacet>(x).value(), 7);
        t.expect("the classic facets are still there", std::has_facet<std::ctype<char>>(x), 1);
        t.expect("_Catmask is 0 after addfac", F<int>(impl(x), 32), 0);
        t.expect("the implementation is a copy: refcount 2 (x and y)", refs(x), 2);
        bool thrown = false;
        try { std::use_facet<MyFacet>(c1); } catch (const std::bad_cast&) { thrown = true; }
        t.check("use_facet of a missing facet throws std::bad_cast", thrown, 0);
        t.value("MyFacet::id after use (lazily assigned index)", (poly_i64)F<unsigned long long>(&MyFacet::id, 0));
    }
    // global()
    {
        std::locale x(c1, new MyFacet);
        std::locale prev = std::locale::global(x);
        t.check("global() returns the previous global locale", prev == c1, 0);
        t.check("locale() is now the new one", std::locale() == x, 0);
        t.check("and std::locale() shares x's implementation", impl(std::locale()) == impl(x), 0);
        std::locale prev2 = std::locale::global(c1);
        t.check("global() again returns x", prev2 == x, 0);
        t.check("restored", std::locale() == c1, 0);
    }
    // a transparent (empty) locale reads the global locale for facets it does not have
    {
        std::locale e = std::locale::empty();
        t.expect("an empty locale is transparent (_Xparent)", F<char>(impl(e), 36), 1);
        t.expect_str("its name is *", e.name().c_str(), "*");
        t.check("use_facet works through the global locale", &std::use_facet<std::ctype<char>>(e) == &std::use_facet<std::ctype<char>>(c1), 0);
        t.expect("_Facetcount is 0", (poly_i64)F<unsigned long long>(impl(e), 24), 0);
    }
    // a bad locale name
    {
        bool thrown = false;
        std::string what;
        try { std::locale bad("no-such-locale-name"); } catch (const std::runtime_error& e) { thrown = true; what = e.what(); }
        t.check("a bad locale name throws std::runtime_error", thrown, 0);
        t.expect_str("its text", what.c_str(), "bad locale name");
        thrown = false;
        try { std::locale bad((const char*)0); } catch (const std::runtime_error& e) { thrown = true; what = e.what(); }
        t.check("a null name throws too", thrown, 0);
        t.expect_str("with the same text", what.c_str(), "bad locale name");
    }
    // facet ids are allocated lazily from _Id_cnt
    {
        void* dll = GetModuleHandleW(L"msvcp140.dll");
        int* idcnt = (int*)GetProcAddress(dll, "?_Id_cnt@id@locale@std@@0HA");
        int before = *idcnt;
        t.value("_Id_cnt is a counter of the allocated ids", before > 0);
        std::locale z(c1, new Other);
        t.expect("a new facet type took the next id", *idcnt, before + 1);
        t.expect("and its id is that number", (poly_i64)F<unsigned long long>(&Other::id, 0), before + 1);
    }
}
POLY_TEST_MAIN(test)
