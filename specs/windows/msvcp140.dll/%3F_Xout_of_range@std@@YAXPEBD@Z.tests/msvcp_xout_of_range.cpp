// poly-cxx: compiler=msvc
// msvcp140.dll std::_Xout_of_range(const char*) (?_Xout_of_range@std@@YAXPEBD@Z): throws std::out_of_range (a std::logic_error) with a copy of the text.
#include "poly_wtest.h"
#include <stdexcept>
#include <new>
#include <functional>
#include <system_error>
#include <typeinfo>

static void test(poly::Test& t) {
    bool caught = false;
    try {
        std::_Xout_of_range("invalid string position");
    } catch (const std::out_of_range& e) {
        caught = true;
        t.expect_str("what() is the text passed in", e.what(), "invalid string position");
        t.check("the dynamic type is exactly std::out_of_range", typeid(e) == typeid(std::out_of_range), 0);
    }
    t.expect("caught as std::out_of_range", caught, 1);
    caught = false;
    try {
        std::_Xout_of_range("invalid vector subscript");
    } catch (const std::logic_error& e) {
        caught = true;
        t.expect_str("what() through std::logic_error", e.what(), "invalid vector subscript");
    }
    t.expect("caught as its base std::logic_error", caught, 1);
    caught = false;
    try {
        std::_Xout_of_range("invalid vector subscript");
    } catch (const std::exception& e) {
        caught = true;
        t.expect_str("what() through std::exception", e.what(), "invalid vector subscript");
    }
    t.expect("caught as its base std::exception", caught, 1);
    caught = false;
    try {
        std::_Xout_of_range("invalid vector subscript");
    } catch (...) {
        caught = true;
    }
    t.expect("caught by catch(...)", caught, 1);
    // The text is copied: the caller's buffer may change or go away after the throw.
    char buf[32] = "volatile text";
    caught = false;
    try {
        try {
            std::_Xout_of_range(buf);
        } catch (const std::out_of_range&) {
            buf[0] = 'X';
            throw;
        }
    } catch (const std::out_of_range& e) {
        caught = true;
        t.expect_str("the message was copied", e.what(), "volatile text");
    }
    t.expect("rethrown", caught, 1);
}
POLY_TEST_MAIN(test)
