// poly-cxx: compiler=msvc
// msvcp140.dll std::_Throw_Cpp_error(int) (?_Throw_Cpp_error@std@@YAXH@Z): throws std::system_error for the codes of xthreads.h (_DEVICE_OR_RESOURCE_BUSY 0 .. _RESOURCE_UNAVAILABLE_TRY_AGAIN 6); the error code is a std::errc of the generic category.
#include "poly_wtest.h"
#include <stdexcept>
#include <new>
#include <functional>
#include <system_error>
#include <typeinfo>

void* __cdecl operator new(unsigned long long n) { return malloc(n); }

static void test(poly::Test& t) {
    static const struct { int code; int errc; } want[] = {
        {0, (int)std::errc::device_or_resource_busy},
        {1, (int)std::errc::invalid_argument},
        {2, (int)std::errc::no_such_process},
        {3, (int)std::errc::not_enough_memory},
        {4, (int)std::errc::operation_not_permitted},
        {5, (int)std::errc::resource_deadlock_would_occur},
        {6, (int)std::errc::resource_unavailable_try_again},
    };
    for (int i = 0; i < 7; i++) {
        bool caught = false;
        try {
            std::_Throw_Cpp_error(want[i].code);
        } catch (const std::system_error& e) {
            caught = true;
            poly::Line l;
            l.str("value _Throw_Cpp_error ").dec(want[i].code).str(" value=").dec(e.code().value()).str(" generic=").dec(e.code().category() == std::generic_category())
                .str(" what=").str(e.what());
            l.end();
            t.expect("the error value of the code", e.code().value(), want[i].errc);
            t.expect("the category is generic", e.code().category() == std::generic_category(), 1);
        }
        t.expect("a system_error was thrown", caught, 1);
    }
}
POLY_TEST_MAIN(test)
