// poly-cxx: compiler=msvc
// msvcp140.dll std::_Xinvalid_argument(const char*) (?_Xinvalid_argument@std@@YAXPEBD@Z): throws std::invalid_argument (a std::logic_error).
#include "poly_wtest.h"
#include <stdexcept>
#include <new>
#include <functional>
#include <system_error>
#include <typeinfo>

static void test(poly::Test& t) {
    bool caught = false;
    try {
        std::_Xinvalid_argument("invalid argument");
    } catch (const std::invalid_argument& e) {
        caught = true;
        t.expect_str("what() is the text passed in", e.what(), "invalid argument");
        t.check("the dynamic type is exactly std::invalid_argument", typeid(e) == typeid(std::invalid_argument), 0);
    }
    t.expect("caught as std::invalid_argument", caught, 1);
    caught = false;
    try {
        std::_Xinvalid_argument("invalid stoi argument");
    } catch (const std::logic_error& e) {
        caught = true;
        t.expect_str("what() through std::logic_error", e.what(), "invalid stoi argument");
    }
    t.expect("caught as its base std::logic_error", caught, 1);
    caught = false;
    try {
        std::_Xinvalid_argument("invalid stoi argument");
    } catch (const std::exception& e) {
        caught = true;
        t.expect_str("what() through std::exception", e.what(), "invalid stoi argument");
    }
    t.expect("caught as its base std::exception", caught, 1);
    caught = false;
    try {
        std::_Xinvalid_argument("invalid stoi argument");
    } catch (...) {
        caught = true;
    }
    t.expect("caught by catch(...)", caught, 1);
    // The text is copied: the caller's buffer may change or go away after the throw.
    char buf[32] = "volatile text";
    caught = false;
    try {
        try {
            std::_Xinvalid_argument(buf);
        } catch (const std::invalid_argument&) {
            buf[0] = 'X';
            throw;
        }
    } catch (const std::invalid_argument& e) {
        caught = true;
        t.expect_str("the message was copied", e.what(), "volatile text");
    }
    t.expect("rethrown", caught, 1);
}
POLY_TEST_MAIN(test)
