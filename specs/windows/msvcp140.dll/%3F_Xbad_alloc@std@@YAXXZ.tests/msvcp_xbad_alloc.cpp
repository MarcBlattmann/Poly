// poly-cxx: compiler=msvc
// msvcp140.dll std::_Xbad_alloc() (?_Xbad_alloc@std@@YAXXZ): throws std::bad_alloc, whose what() is "bad allocation".
#include "poly_wtest.h"
#include <stdexcept>
#include <new>
#include <functional>
#include <system_error>
#include <typeinfo>

static void test(poly::Test& t) {
    bool caught = false;
    try {
        std::_Xbad_alloc();
    } catch (const std::bad_alloc& e) {
        caught = true;
        t.expect_str("what()", e.what(), "bad allocation");
        t.check("exactly bad_alloc", typeid(e) == typeid(std::bad_alloc), 0);
    }
    t.expect("caught as std::bad_alloc", caught, 1);
    caught = false;
    try {
        std::_Xbad_alloc();
    } catch (const std::exception& e) {
        caught = true;
        t.expect_str("what() through std::exception", e.what(), "bad allocation");
    }
    t.expect("caught as std::exception", caught, 1);
    caught = false;
    try {
        std::_Xbad_alloc();
    } catch (const std::bad_array_new_length&) {
    } catch (const std::bad_alloc&) {
        caught = true;
    }
    t.expect("it is not a bad_array_new_length", caught, 1);
}
POLY_TEST_MAIN(test)
