// poly-cxx: compiler=msvc
// msvcp140.dll std::_Xruntime_error(const char*) and std::_Xoverflow_error(const char*): std::runtime_error and std::overflow_error (a std::runtime_error).
#include "poly_wtest.h"
#include <stdexcept>
#include <new>
#include <functional>
#include <system_error>
#include <typeinfo>

static void test(poly::Test& t) {
    bool caught = false;
    try {
        std::_Xruntime_error("runtime failure");
    } catch (const std::runtime_error& e) {
        caught = true;
        t.expect_str("what() is the text passed in", e.what(), "runtime failure");
        t.check("the dynamic type is exactly std::runtime_error", typeid(e) == typeid(std::runtime_error), 0);
    }
    t.expect("caught as std::runtime_error", caught, 1);
    caught = false;
    try {
        std::_Xruntime_error("another failure");
    } catch (const std::exception& e) {
        caught = true;
        t.expect_str("what() through std::exception", e.what(), "another failure");
    }
    t.expect("caught as its base std::exception", caught, 1);
    caught = false;
    try {
        std::_Xruntime_error("another failure");
    } catch (...) {
        caught = true;
    }
    t.expect("caught by catch(...)", caught, 1);
    // The text is copied: the caller's buffer may change or go away after the throw.
    char buf[32] = "volatile text";
    caught = false;
    try {
        try {
            std::_Xruntime_error(buf);
        } catch (const std::runtime_error&) {
            buf[0] = 'X';
            throw;
        }
    } catch (const std::runtime_error& e) {
        caught = true;
        t.expect_str("the message was copied", e.what(), "volatile text");
    }
    t.expect("rethrown", caught, 1);
    caught = false;
    try {
        std::_Xoverflow_error("too big");
    } catch (const std::overflow_error& e) {
        caught = true;
        t.expect_str("overflow_error what()", e.what(), "too big");
        t.check("exactly overflow_error", typeid(e) == typeid(std::overflow_error), 0);
    }
    t.expect("overflow_error caught", caught, 1);
    caught = false;
    try {
        std::_Xoverflow_error("too big");
    } catch (const std::runtime_error&) {
        caught = true;
    }
    t.expect("overflow_error is a runtime_error", caught, 1);
}
POLY_TEST_MAIN(test)
