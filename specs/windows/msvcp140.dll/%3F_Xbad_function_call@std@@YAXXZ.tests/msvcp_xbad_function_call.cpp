// poly-cxx: compiler=msvc
// msvcp140.dll std::_Xbad_function_call() (?_Xbad_function_call@std@@YAXXZ): throws std::bad_function_call, what() "bad function call".
#include "poly_wtest.h"
#include <stdexcept>
#include <new>
#include <functional>
#include <system_error>
#include <typeinfo>

static void test(poly::Test& t) {
    bool caught = false;
    try {
        std::_Xbad_function_call();
    } catch (const std::bad_function_call& e) {
        caught = true;
        t.expect_str("what()", e.what(), "bad function call");
        t.check("exactly bad_function_call", typeid(e) == typeid(std::bad_function_call), 0);
    }
    t.expect("caught", caught, 1);
    caught = false;
    try {
        std::_Xbad_function_call();
    } catch (const std::exception& e) {
        caught = true;
    }
    t.expect("it is a std::exception", caught, 1);
}
POLY_TEST_MAIN(test)
