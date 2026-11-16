// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::_Winerror_map (?_Winerror_map@std@@YAHH@Z): the map from Windows error codes to generic (errno) values, 0 for no equivalent.
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
extern "C" size_t __stdcall __std_system_error_allocate_message(unsigned long, char** p) noexcept { *p = 0; return 0; }
extern "C" void __stdcall __std_system_error_deallocate_message(char*) noexcept {}
namespace std { _CRTIMP2_PURE int __cdecl _Winerror_map(int); }
struct Pair { int win, gen; };
static const Pair map[] = {
    {1, 40}, {2, 2}, {3, 2}, {4, 24}, {5, 13}, {6, 22}, {8, 12}, {12, 13}, {14, 12}, {15, 19}, {16, 13}, {17, 18}, {19, 13}, {20, 19},
    {21, 11}, {25, 5}, {29, 5}, {30, 5}, {32, 13}, {33, 39}, {39, 28}, {50, 129}, {53, 2}, {55, 19}, {67, 2}, {80, 17}, {82, 13}, {87, 22},
    {109, 32}, {110, 5}, {111, 38}, {112, 28}, {121, 138}, {123, 2}, {131, 22}, {142, 16}, {145, 41}, {170, 16}, {183, 17}, {206, 38}, {212, 39},
    {258, 138}, {267, 22}, {995, 105}, {998, 13}, {1011, 5}, {1012, 5}, {1013, 5}, {1237, 11}, {1460, 138}, {2401, 16}, {2404, 16}, {4393, 22},
    {10004, 4}, {10009, 9}, {10013, 13}, {10014, 14}, {10022, 22}, {10024, 24}, {10035, 140}, {10036, 112}, {10037, 103}, {10038, 128},
    {10039, 109}, {10040, 115}, {10041, 136}, {10042, 123}, {10043, 135}, {10045, 130}, {10047, 102}, {10048, 100}, {10049, 101}, {10050, 116},
    {10051, 118}, {10052, 117}, {10053, 106}, {10054, 108}, {10055, 119}, {10056, 113}, {10057, 126}, {10060, 138}, {10061, 107}, {10063, 38},
    {10065, 110},
};
static void g1(poly::Test& t) {
    int bad = 0;
    for (unsigned i = 0; i < sizeof map / sizeof map[0]; i++) {
        int got = std::_Winerror_map(map[i].win);
        poly::Line l; l.str("value winerror ").dec(map[i].win).str(" => ").dec(got); l.end();
        if (got != map[i].gen) bad++;
    }
    t.expect("every listed code maps as observed on Windows", bad, 0);
    t.expect("the table has 84 entries", (poly_i64)(sizeof map / sizeof map[0]), 84);
}
static void g2(poly::Test& t) {
    int nonzero = 0;
    for (int i = -5; i < 70000; i++) {
        int m = std::_Winerror_map(i);
        if (m != 0) nonzero++;
    }
    t.expect("exactly 84 codes in -5..69999 have a mapping", nonzero, 84);
    t.expect("0 maps to 0", std::_Winerror_map(0), 0);
    t.expect("1314 (privilege not held) has no mapping", std::_Winerror_map(1314), 0);
    t.expect("negative values have none", std::_Winerror_map(-1), 0);
    t.expect("large values have none", std::_Winerror_map(0x7fffffff), 0);
}
static void test(poly::Test& t) {
    g1(t);
    g2(t);
}
POLY_TEST_MAIN(test)
