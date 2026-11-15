// poly-cxx: compiler=msvc
// msvcp140.dll std::_Xlength_error(const char*) (?_Xlength_error@std@@YAXPEBD@Z): throws std::length_error (a std::logic_error, a std::exception)
// carrying a copy of the text; what() returns it.
#include "poly_wtest.h"
#include <stdexcept>
#include <new>
#include <functional>
#include <system_error>
#include <typeinfo>

static void test(poly::Test& t) {
    bool caught = false;
    try {
        std::_Xlength_error("string too long");
    } catch (const std::length_error& e) {
        caught = true;
        t.expect_str("what() is the text passed in", e.what(), "string too long");
        t.check("the dynamic type is exactly std::length_error", typeid(e) == typeid(std::length_error), 0);
    }
    t.expect("caught as std::length_error", caught, 1);
    caught = false;
    try {
        std::_Xlength_error("vector too long");
    } catch (const std::logic_error& e) {
        caught = true;
        t.expect_str("what() through std::logic_error", e.what(), "vector too long");
    }
    t.expect("caught as its base std::logic_error", caught, 1);
    caught = false;
    try {
        std::_Xlength_error("vector too long");
    } catch (const std::exception& e) {
        caught = true;
        t.expect_str("what() through std::exception", e.what(), "vector too long");
    }
    t.expect("caught as its base std::exception", caught, 1);
    caught = false;
    try {
        std::_Xlength_error("vector too long");
    } catch (...) {
        caught = true;
    }
    t.expect("caught by catch(...)", caught, 1);
    // The text is copied: the caller's buffer may change or go away after the throw.
    char buf[32] = "volatile text";
    caught = false;
    try {
        try {
            std::_Xlength_error(buf);
        } catch (const std::length_error&) {
            buf[0] = 'X';
            throw;
        }
    } catch (const std::length_error& e) {
        caught = true;
        t.expect_str("the message was copied", e.what(), "volatile text");
    }
    t.expect("rethrown", caught, 1);
}
POLY_TEST_MAIN(test)
