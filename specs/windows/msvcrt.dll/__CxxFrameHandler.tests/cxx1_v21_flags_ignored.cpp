// poly-cxx: handler=__CxxFrameHandler magic=0x19930521 ehflags=5
// __CxxFrameHandler, magic 0x19930521 with a 5 where 0x19930522 has its EH
// flags: a 0x19930521 FuncInfo has no flags field, so the "noexcept" bit is
// not seen and the exception is caught normally.
#include "poly_wtest.h"

static poly::Log g_log;

struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

static void on_terminate() {
    g_log.add("terminate");
    poly::g_test.expect_str("order", g_log.str(), "terminate was not expected");
    poly::finish_and_exit();
}
__declspec(noinline) static void inner() {
    A a(1);
    throw 2;
}
static void test(poly::Test& t) {
    set_terminate(on_terminate);
    int r = 0;
    try {
        inner();
    } catch (int x) {
        r = x;
    }
    t.expect("caught", r, 2);
    t.expect_str("order", g_log.str(), "A+1 ~A1");
}
POLY_TEST_MAIN(test)
