// __CxxFrameHandler3: nested try blocks. An inner try block whose clauses
// do not match passes the exception to the enclosing try block of the same
// function (the try block map: TryLow/TryHigh/CatchHigh); a try block inside
// a catch block catches exceptions thrown there while the outer exception
// is being handled, and the outer exception stays valid; an exception
// thrown from a nested catch block is caught by an outer try block of the
// same function.
#include "poly_wtest.h"

static poly::Log g_log;

struct X {
    int v;
    X(int x) : v(x) { g_log.add_num("X+", v); }
    X(const X& o) : v(o.v) { g_log.add_num("Xcopy", v); }
    ~X() { g_log.add_num("~X", v); }
};

__declspec(noinline) static void throw_x(int v) { throw X(v); }
__declspec(noinline) static void throw_int(int v) { throw v; }

static void test(poly::Test& t) {
    try {
        try {
            throw_int(1);
        } catch (X&) {
            g_log.add("inner");
        }
        g_log.add("not here");
    } catch (int v) {
        g_log.add_num("outer", v);
    }
    t.expect_str("inner try block does not match: the outer one does", g_log.str(), "outer1");
    g_log.clear();

    try {
        try {
            throw_int(2);
        } catch (int v) {
            g_log.add_num("inner", v);
        }
        g_log.add("after inner");
    } catch (int) {
        g_log.add("outer");
    }
    t.expect_str("inner try block matches", g_log.str(), "inner2 after inner");
    g_log.clear();

    try {
        throw_x(3);
    } catch (X& outer) {
        try {
            throw_int(4);
        } catch (int v) {
            g_log.add_num("nested", v);
        }
        g_log.add_num("outer still", outer.v);
    }
    t.expect_str("try block in a catch block", g_log.str(), "X+3 nested4 outer still3 ~X3");
    g_log.clear();

    try {
        try {
            throw_x(5);
        } catch (X& a) {
            try {
                throw_x(6);
            } catch (X& b) {
                g_log.add_num("level2", a.v * 10 + b.v);
                throw_int(7);
            }
        }
    } catch (int v) {
        g_log.add_num("outermost", v);
    }
    t.expect_str("throw from a catch block two levels deep, caught by the outer try block", g_log.str(),
                 "X+5 X+6 level256 ~X6 ~X5 outermost7");
    g_log.clear();

    int n = 0;
    for (int i = 0; i < 3; i++) {
        try {
            throw_int(i);
        } catch (int v) {
            n += v + 1;
        }
        try {
            throw_x(i);
        } catch (X& x) {
            n += 10 * (x.v + 1);
        }
    }
    t.expect("sequential try blocks in a loop", n, 66);
}
POLY_TEST_MAIN(test)
