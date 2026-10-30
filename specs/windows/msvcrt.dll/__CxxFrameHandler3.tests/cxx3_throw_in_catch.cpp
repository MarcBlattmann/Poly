// __CxxFrameHandler3: a new exception thrown in a catch block replaces the
// one being handled. The first exception object is destroyed while the
// catch block is unwound, after the new one was made and before the clause
// that takes the new one runs; that clause may be in an enclosing try block
// of the same function or in a caller.
#include "poly_wtest.h"

static poly::Log g_log;

struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

__declspec(noinline) static void throw_a(int v) { throw A(v); }

__declspec(noinline) static void replace_in_catch() {
    try {
        throw_a(1);
    } catch (A& a) {
        g_log.add_num("c1", a.v);
        throw A(2);
    }
}

__declspec(noinline) static void replace_by_value() {
    A local(5);
    try {
        throw_a(6);
    } catch (A a) {
        g_log.add_num("c", a.v);
        throw 7;
    }
}

static void test(poly::Test& t) {
    try {
        replace_in_catch();
    } catch (A& b) {
        g_log.add_num("c2", b.v);
    }
    t.expect_str("first object destroyed before the clause of the second runs", g_log.str(),
                 "A+1 c11 A+2 ~A1 c22 ~A2");
    g_log.clear();

    try {
        try {
            throw_a(3);
        } catch (A& a) {
            g_log.add_num("inner", a.v);
            throw_a(4);
        }
    } catch (A& b) {
        g_log.add_num("outer", b.v);
    }
    t.expect_str("caught by the enclosing try block of the same function", g_log.str(),
                 "A+3 inner3 A+4 ~A3 outer4 ~A4");
    g_log.clear();

    try {
        replace_by_value();
    } catch (int v) {
        g_log.add_num("int", v);
    }
    t.expect_str("catch object, exception object, then the frame's locals", g_log.str(),
                 "A+5 A+6 Acopy106 c106 ~A106 ~A6 ~A5 int7");
}
POLY_TEST_MAIN(test)
