// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::_Throw_C_error (?_Throw_C_error@std@@YAXH@Z) and std::_XGetLastError (?_XGetLastError@std@@YAXXZ): the exceptions they throw and the system category of the library.
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
#include <functional>
extern "C" __declspec(dllimport) void __stdcall SetLastError(unsigned long);
namespace std { _CRTIMP2_PURE unsigned long __cdecl _Winerror_message(unsigned long, char*, unsigned long); _CRTIMP2_PURE void __cdecl _Throw_C_error(int); _CRTIMP2_PURE void __cdecl _XGetLastError(); }
static unsigned long long addr_of(const std::error_category& c) { return *(const unsigned long long*)((const char*)&c + 8); }
struct OB : std::streambuf { int overflow(int c) override { return c; } };
static void h1(poly::Test& t) {
    for (int c = 1; c < 5; c++) {
        poly::Line l; l.str("value throw_c ").dec(c).str(" => ");
        try { std::_Throw_C_error(c); l.str("no throw"); }
        catch (const std::system_error& e) { l.str("system_error value=").dec(e.code().value()).str(" category=").dec((poly_i64)addr_of(e.code().category())).str(" what=").str(e.what()); }
        catch (...) { l.str("other"); }
        l.end();
    }
    int got[5] = {0, 0, 0, 0, 0};
    for (int c = 1; c < 5; c++) {
        try { std::_Throw_C_error(c); } catch (const std::system_error& e) { got[c] = e.code().value(); }
    }
    t.expect("_Throw_C_error(1) (_Nomem)", got[1], 11);
    t.expect("_Throw_C_error(2) (_Timedout)", got[2], 11);
    t.expect("_Throw_C_error(3) (_Busy)", got[3], 16);
    t.expect("_Throw_C_error(4) (_Error)", got[4], 22);
    bool generic = false;
    try { std::_Throw_C_error(3); } catch (const std::system_error& e) { generic = e.code().category() == std::generic_category(); }
    t.check("the category is the generic one", generic, 0);
}
static void h2(poly::Test& t) {
    SetLastError(5);
    try { std::_XGetLastError(); t.check("_XGetLastError throws", false, 0); }
    catch (const std::system_error& e) {
        t.expect("the error value is GetLastError()", e.code().value(), 5);
        t.expect("the identity word of the category is 7 (system)", (poly_i64)addr_of(e.code().category()), 7);
        t.check("it equals the program's system_category()", e.code().category() == std::system_category(), 0);
        t.expect_str("name() through the library's category object", e.code().category().name(), "system");
        t.expect_str("what() is the system message of the code", e.what(), "Access is denied.");
        std::error_condition ec = e.code().category().default_error_condition(5);
        t.expect("default_error_condition(5) through the library's category: value", ec.value(), 13);
        t.check("default_error_condition(5): generic", ec.category() == std::generic_category(), 0);
        t.check("equivalent(int, error_condition) is true for permission_denied", e.code().category().equivalent(5, std::make_error_condition(std::errc::permission_denied)), 0);
        t.check("equivalent(error_code, int): the code itself", e.code().category().equivalent(e.code(), 5), 0);
        t.check("equivalent(error_code, int): another value is false", !e.code().category().equivalent(e.code(), 6), 0);
        t.expect_str("message(2) through the library's category object", e.code().category().message(2).c_str(), "The system cannot find the file specified.");
        t.expect_str("message(99999)", e.code().category().message(99999).c_str(), "unknown error");
    }
    SetLastError(0);
    try { std::_XGetLastError(); }
    catch (const std::system_error& e) {
        t.expect("GetLastError() 0 gives value 0", e.code().value(), 0);
        t.expect_str("and the text of success", e.what(), "The operation completed successfully.");
    }
}
static void test(poly::Test& t) {
    h1(t);
    h2(t);
}
POLY_TEST_MAIN(test)
