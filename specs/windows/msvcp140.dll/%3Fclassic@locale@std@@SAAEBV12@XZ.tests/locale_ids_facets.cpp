// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::locale::id and the exported facet ids (?id@?$ctype@D@std@@2V0locale@2@A, ?id@?$numpunct@D@std@@2V0locale@2@A, ?id@?$collate@D@std@@2V0locale@2@A, ?id@?$codecvt@DDU_Mbstatet@@@std@@2V0locale@2@A, ?id@?$num_get@D..., ?id@?$num_put@D..., ?id@?$time_get@D..., ?id@?$time_put@D..., ?id@?$moneypunct@D..., ?id@?$money_get@D..., ?id@?$money_put@D..., ?id@?$messages@D@std@@2V0locale@2@A and the wide ones), ??0id@locale@std@@QEAA@_K@Z, ??Bid@locale@std@@QEAA_KXZ, ?_Id_cnt@id@locale@std@@0HA: which index each standard facet holds when the program starts, and the lazily allocated ones.
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
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
template <class T> static poly_i64 idof() { return (poly_i64)F<unsigned long long>(&T::id, 0); }
#define ID(name, ...) t.value(name, idof<__VA_ARGS__>())
static void g1(poly::Test& t) {
    ID("ctype<char>::id", std::ctype<char>);
    ID("ctype<wchar_t>::id", std::ctype<wchar_t>);
    ID("ctype<unsigned short>::id", std::ctype<unsigned short>);
    ID("collate<char>::id", std::collate<char>);
    ID("collate<wchar_t>::id", std::collate<wchar_t>);
    ID("numpunct<char>::id", std::numpunct<char>);
    ID("numpunct<wchar_t>::id", std::numpunct<wchar_t>);
    ID("num_get<char>::id", std::num_get<char>);
    ID("num_put<char>::id", std::num_put<char>);
    ID("num_get<wchar_t>::id", std::num_get<wchar_t>);
    ID("num_put<wchar_t>::id", std::num_put<wchar_t>);
    ID("time_get<char>::id", std::time_get<char>);
    ID("time_put<char>::id", std::time_put<char>);
    ID("time_get<wchar_t>::id", std::time_get<wchar_t>);
    ID("time_put<wchar_t>::id", std::time_put<wchar_t>);
    ID("moneypunct<char, false>::id", std::moneypunct<char, false>);
    ID("moneypunct<char, true>::id", std::moneypunct<char, true>);
    ID("money_get<char>::id", std::money_get<char>);
    ID("money_put<char>::id", std::money_put<char>);
    ID("messages<char>::id", std::messages<char>);
    t.check("codecvt<char, char, mbstate_t>::id is a facet id as well", std::codecvt<char, char, _Mbstatet>::id._Get_index() != 0, 0);
    ID("codecvt<char, char, mbstate_t>::id", std::codecvt<char, char, _Mbstatet>);
    ID("codecvt<wchar_t, char, mbstate_t>::id", std::codecvt<wchar_t, char, _Mbstatet>);
}
static void g2(poly::Test& t) {
    t.expect("ctype<char>::id (exported by the library) is 1", idof<std::ctype<char>>(), 1);
    t.expect("ctype<unsigned short>::id is 2", idof<std::ctype<unsigned short>>(), 2);
    t.expect("ctype<wchar_t>::id is 3", idof<std::ctype<wchar_t>>(), 3);
    t.expect("codecvt<char, char, mbstate_t>::id is 4", idof<std::codecvt<char, char, _Mbstatet>>(), 4);
    t.expect("the ids of facet classes that are built into the program (numpunct, collate, ...) are 0 until first used", idof<std::numpunct<char>>() + idof<std::collate<char>>() + idof<std::messages<char>>(), 0);
    size_t n = std::numpunct<char>::id._Get_index();
    t.check("using one allocates the next number after the library's own", n > 4, 0);
    t.expect("and the id then holds it", idof<std::numpunct<char>>(), (poly_i64)n);
}
struct Fresh2 : std::locale::facet {
    static std::locale::id id;
    Fresh2() : facet(0) {}
};
std::locale::id Fresh2::id;
struct Fresh3 : std::locale::facet {
    static std::locale::id id;
    Fresh3() : facet(0) {}
};
std::locale::id Fresh3::id;
static void g3(poly::Test& t) {
    t.expect("a fresh id holds 0", (poly_i64)F<unsigned long long>(&Fresh2::id, 0), 0);
    size_t a = Fresh2::id._Get_index();
    size_t b = Fresh3::id._Get_index();
    t.expect("the next index is one more than the last", (poly_i64)(b - a), 1);
    t.expect("asking again returns the same index", (poly_i64)(Fresh2::id._Get_index() - a), 0);
    t.value("sizeof(locale::id)", sizeof(std::locale::id));
}
static void test(poly::Test& t) {
    g1(t);
    g2(t);
    g3(t);
}
POLY_TEST_MAIN(test)
