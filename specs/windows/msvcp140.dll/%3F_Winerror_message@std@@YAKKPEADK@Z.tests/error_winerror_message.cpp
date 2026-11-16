// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::_Winerror_message (?_Winerror_message@std@@YAKKPEADK@Z): the text of a Windows error code.
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
    char buf[256];
    static const unsigned long codes[] = {0, 2, 5, 87, 1314, 99999};
    for (unsigned i = 0; i < 6; i++) {
        unsigned long n = std::_Winerror_message(codes[i], buf, 255);
        poly::Line l; l.str("value winerror_message ").dec(codes[i]).str(" len=").dec(n).str(" text=");
        buf[n < 255 ? n : 255] = 0;
        l.str(buf); l.end();
    }
    unsigned long n = std::_Winerror_message(5, buf, 255);
    t.expect("the message of 5 is 17 characters", n, 17);
    t.expect_hex("the character after the message is the carriage return of the system text", (unsigned char)buf[n], 0x0D);
    buf[n] = 0;
    t.expect_str("the message of 5", buf, "Access is denied.");
    n = std::_Winerror_message(2, buf, 255);
    buf[n] = 0;
    t.expect_str("the message of 2", buf, "The system cannot find the file specified.");
    n = std::_Winerror_message(99999, buf, 255);
    t.expect("an unknown code gives length 0", n, 0);
    // a short buffer
    memset(buf, 'x', sizeof buf);
    n = std::_Winerror_message(5, buf, 8);
    poly::Line l; l.str("value winerror_message into a buffer of 8: len=").dec(n); l.end();
}
static void test(poly::Test& t) {
    h1(t);
}
POLY_TEST_MAIN(test)
