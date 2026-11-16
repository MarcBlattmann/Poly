// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::_Syserror_map (?_Syserror_map@std@@YAPEBDH@Z) as used by generic_category(): its identity word, name, messages and error_code comparisons (the category object itself is built into the program; its text comes from _Syserror_map).
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
namespace std { _CRTIMP2_PURE const char* __cdecl _Syserror_map(int); }
static void g1(poly::Test& t) {
    const std::error_category& g = std::generic_category();
    for (int i = 0; i < 140; i += 7) {
        t.check("generic_category().message(n) is _Syserror_map(n)", g.message(i) == std::_Syserror_map(i), i);
    }
    t.expect_str("message(0)", g.message(0).c_str(), "success");
    t.expect_str("message(2)", g.message(2).c_str(), "no such file or directory");
    t.expect_str("message(13)", g.message(13).c_str(), "permission denied");
    t.expect_str("message(22)", g.message(22).c_str(), "invalid argument");
    t.expect_str("message(12)", g.message(12).c_str(), "not enough memory");
    t.expect_str("message(17)", g.message(17).c_str(), "file exists");
    t.expect_str("message(28)", g.message(28).c_str(), "no space on device");
    t.expect_str("message(34)", g.message(34).c_str(), "result out of range");
    t.expect_str("message(36)", g.message(36).c_str(), "resource deadlock would occur");
    t.expect_str("message(9999)", g.message(9999).c_str(), "unknown error");
    t.expect_str("name", g.name(), "generic");
    t.expect("the generic category's identity word is 3", (poly_i64)*(unsigned long long*)((char*)&g + 8), 3);
    t.expect("an error_category is 16 bytes (vptr + identity)", (poly_i64)sizeof(std::error_category), 16);
    t.expect("error_code is 16 bytes (value, category pointer)", (poly_i64)sizeof(std::error_code), 16);
    t.check("generic_category() returns the same object every time", &std::generic_category() == &g, 0);
    std::error_code ec(2, g);
    t.expect_str("error_code::message()", ec.message().c_str(), "no such file or directory");
    t.check("error_code(2, generic) == errc::no_such_file_or_directory", ec == std::errc::no_such_file_or_directory, 0);
    t.check("... and != errc::permission_denied", ec != std::errc::permission_denied, 0);
    t.check("make_error_code(errc) uses the generic category", std::make_error_code(std::errc::invalid_argument).category() == g, 0);
    t.expect("and the generic value of errc::invalid_argument is 22", std::make_error_code(std::errc::invalid_argument).value(), 22);
}
static void test(poly::Test& t) {
    g1(t);
}
POLY_TEST_MAIN(test)
