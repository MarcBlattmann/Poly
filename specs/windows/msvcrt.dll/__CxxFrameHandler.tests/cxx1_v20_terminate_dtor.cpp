// poly-cxx: handler=__CxxFrameHandler magic=0x19930520
// __CxxFrameHandler, magic 0x19930520: an exception leaving a destructor
// during an unwind makes the runtime call terminate().
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
    poly::g_test.expect_str("order", g_log.str(), "A+1 ~Bad terminate");
    poly::g_test.expect("by the runtime", poly_std_terminate_calls, 0);
    poly::finish_and_exit();
}
struct Bad {
    ~Bad() noexcept(false) {
        g_log.add("~Bad");
        throw 2;
    }
};
__declspec(noinline) static void f() {
    A a(1);
    Bad b;
    throw 1;
}
static void test(poly::Test& t) {
    set_terminate(on_terminate);
    g_log.clear();
    try {
        f();
    } catch (int) {
        g_log.add("caught");
    }
    t.expect_str("not reached", g_log.str(), "terminate before this");
}
POLY_TEST_MAIN(test)
