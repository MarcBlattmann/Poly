// __CxxFrameHandler3: an exception that leaves a destructor while that
// destructor runs as part of an unwind (here a noexcept(false) destructor,
// so the compiler adds no terminate scope of its own) makes the runtime
// call terminate(), i.e. the handler set_terminate installed. Nothing else
// is unwound: the remaining destructors do not run and no catch clause is
// entered.
#include "poly_wtest.h"

static poly::Log g_log;

static void on_terminate() {
    g_log.add("terminate");
    poly::g_test.expect_str("order", g_log.str(), "~Bad terminate");
    poly::g_test.expect("the runtime called terminate, not the compiler's scope", poly_std_terminate_calls, 0);
    poly::finish_and_exit();
}

struct Bad {
    ~Bad() noexcept(false) {
        g_log.add("~Bad");
        throw 2;
    }
};
struct D {
    ~D() { g_log.add("~D"); }
};

__declspec(noinline) static void f() {
    D d;
    Bad b;
    throw 1;
}

static void test(poly::Test& t) {
    set_terminate(on_terminate);
    try {
        f();
    } catch (int x) {
        g_log.add_num("caught", x);
    }
    t.expect_str("not reached", g_log.str(), "terminate before this");
}
POLY_TEST_MAIN(test)
