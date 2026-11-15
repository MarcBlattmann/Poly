// poly-cxx: compiler=msvc
// msvcp140.dll std::uncaught_exceptions() (?uncaught_exceptions@std@@YAHXZ) and std::uncaught_exception()
// (?uncaught_exception@std@@YA_NXZ): the number of exceptions thrown and not yet caught on this thread (C++17), and whether
// there is at least one. 0 in normal code and in a catch block, 1 in a destructor that runs during an unwind, 2 while a
// second exception unwinds a destructor started by a first one that a catch block is still handling is NOT counted
// (the first one is caught).
#include "poly_wtest.h"
#include <exception>

static poly::Log g_log;

struct Probe {
    const char* name;
    Probe(const char* n) : name(n) {}
    ~Probe() {
        g_log.add_num(name, std::uncaught_exceptions());
        g_log.add(std::uncaught_exception() ? "uncaught" : "none");
    }
};

__declspec(noinline) static void thrower() {
    Probe p("unwinding");
    throw 1;
}

__declspec(noinline) static void normal_exit() {
    Probe p("normal");
}

__declspec(noinline) static void nested() {
    Probe outer("outer");
    try {
        Probe inner("inner");
        throw 2;
    } catch (int) {
        g_log.add_num("in-catch", std::uncaught_exceptions());
        try {
            thrower();
        } catch (int) {
            g_log.add_num("in-inner-catch", std::uncaught_exceptions());
        }
    }
}

static void test(poly::Test& t) {
    t.expect("no exception", std::uncaught_exceptions(), 0);
    t.expect("uncaught_exception() is false", std::uncaught_exception(), 0);
    normal_exit();
    t.expect_str("a destructor of a normal exit", g_log.str(), "normal0 none");
    g_log.clear();
    try {
        thrower();
    } catch (int) {
        t.expect("none in a catch block", std::uncaught_exceptions(), 0);
    }
    t.expect_str("a destructor during an unwind", g_log.str(), "unwinding1 uncaught");
    g_log.clear();
    nested();
    t.expect_str("nested", g_log.str(), "inner1 uncaught in-catch0 unwinding1 uncaught in-inner-catch0 outer0 none");
    t.expect("none left", std::uncaught_exceptions(), 0);
}
POLY_TEST_MAIN(test)
