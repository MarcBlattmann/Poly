// poly-cxx: handler=__CxxFrameHandler
// __CxxFrameHandler with FuncInfo magic 0x19930522 (EH flags 1), what
// __CxxFrameHandler3 also reads: the same rules through the older entry
// point.
#include "poly_wtest.h"

static poly::Log g_log;

struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

__declspec(noinline) static void throw_int(int v) {
    A guard(v);
    throw v;
}
__declspec(noinline) static void throw_a(int v) {
    A local(v);
    throw A(v + 1);
}
__declspec(noinline) static void level2(int v) {
    A a(v);
    throw_int(v + 1);
}
__declspec(noinline) static void level1(int v) {
    A a(v);
    level2(v + 1);
}

static void test(poly::Test& t) {
    int r = 0;
    try {
        throw_int(1);
    } catch (long) {
        r = -1;
    } catch (int x) {
        r = x;
    } catch (...) {
        r = -2;
    }
    t.expect("int clause after a long clause", r, 1);
    t.expect_str("guard destroyed", g_log.str(), "A+1 ~A1");
    g_log.clear();

    try {
        throw_a(10);
    } catch (A a) {
        g_log.add_num("catch", a.v);
    }
    t.expect_str("by value: copy before unwind, param then object", g_log.str(),
                 "A+10 A+11 Acopy111 ~A10 catch111 ~A111 ~A11");
    g_log.clear();

    try {
        throw_a(20);
    } catch (A& a) {
        g_log.add_num("catch", a.v);
    }
    t.expect_str("by reference: no copy", g_log.str(), "A+20 A+21 ~A20 catch21 ~A21");
    g_log.clear();

    try {
        level1(30);
    } catch (int x) {
        g_log.add_num("catch", x);
    }
    t.expect_str("three frames unwound innermost first", g_log.str(), "A+30 A+31 A+32 ~A32 ~A31 ~A30 catch32");
    g_log.clear();

    try {
        throw_a(40);
    } catch (...) {
        g_log.add("all");
    }
    t.expect_str("catch(...) takes a class", g_log.str(), "A+40 A+41 ~A40 all ~A41");
}
POLY_TEST_MAIN(test)
