// poly-cxx: handler=__CxxFrameHandler magic=0x19930521
// __CxxFrameHandler, magic 0x19930521: rethrow and nesting as with the
// other layouts.
#include "poly_wtest.h"

static poly::Log g_log;

struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

__declspec(noinline) static void throw_a(int v) { throw A(v); }

__declspec(noinline) static void rethrower() {
    try {
        throw_a(1);
    } catch (A&) {
        g_log.add("inner");
        throw;
    }
}
__declspec(noinline) static void replacer() {
    try {
        throw_a(2);
    } catch (A&) {
        g_log.add("inner");
        throw A(3);
    }
}

static void test(poly::Test& t) {
    try {
        rethrower();
    } catch (A& a) {
        g_log.add_num("outer", a.v);
    }
    t.expect_str("rethrow: same object, destroyed once", g_log.str(), "A+1 inner outer1 ~A1");
    g_log.clear();

    try {
        replacer();
    } catch (A& a) {
        g_log.add_num("outer", a.v);
    }
    t.expect_str("throw in a catch block", g_log.str(), "A+2 inner A+3 ~A2 outer3 ~A3");
    g_log.clear();

    try {
        try {
            throw_a(4);
        } catch (int) {
            g_log.add("wrong");
        }
    } catch (A& a) {
        g_log.add_num("enclosing", a.v);
    }
    t.expect_str("enclosing try block of the same function", g_log.str(), "A+4 enclosing4 ~A4");
    g_log.clear();

    try {
        throw_a(5);
    } catch (A& a) {
        try {
            throw 6;
        } catch (int x) {
            g_log.add_num("nested", x);
        }
        g_log.add_num("still", a.v);
    }
    t.expect_str("try block inside a catch block", g_log.str(), "A+5 nested6 still5 ~A5");
}
POLY_TEST_MAIN(test)
