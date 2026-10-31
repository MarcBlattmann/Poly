// __CxxFrameHandler3: a reference catch object is the exception object
// itself (no copy). That object lives where the throw made it, in the
// throwing function's frame, which is deeper in the stack than the catching
// function: the frames between are unwound (destructors run) but their
// stack memory is still in use while the catch block runs. The object is
// destroyed once, when the catch block ends; a change made through the
// reference is what a rethrow passes on.
#include "poly_wtest.h"

static poly::Log g_log;
static const void* g_made;

struct E {
    int v;
    E(int x) : v(x) {
        g_made = this;
        g_log.add_num("E+", v);
    }
    E(const E& o) : v(o.v) { g_log.add("Ecopy"); }
    ~E() { g_log.add_num("~E", v); }
};

struct Guard {
    ~Guard() { g_log.add("~Guard"); }
};

__declspec(noinline) static void thrower(int v) {
    Guard g;
    throw E(v);
}

__declspec(noinline) static int modify_and_rethrow() {
    try {
        thrower(3);
    } catch (E& e) {
        e.v = 30;
        throw;
    }
    return 0;
}

static void test(poly::Test& t) {
    int local = 0;
    try {
        thrower(1);
    } catch (E& e) {
        t.check("the reference is the exception object", (const void*)&e == g_made, 0);
        t.check("the object is deeper in the stack than the catcher", (const char*)&e < (const char*)&local, 0);
        g_log.add_num("catch", e.v);
    }
    t.expect_str("no copy, destroyed at the end of the catch block", g_log.str(), "E+1 ~Guard catch1 ~E1");
    g_log.clear();

    try {
        thrower(2);
    } catch (const E& e) {
        g_log.add_num("catch", e.v);
    }
    t.expect_str("const reference too", g_log.str(), "E+2 ~Guard catch2 ~E2");
    g_log.clear();

    int seen = 0;
    try {
        modify_and_rethrow();
    } catch (E& e) {
        seen = e.v;
        t.check("the rethrown object is the same object", (const void*)&e == g_made, 0);
    }
    t.expect("a change through the reference survives the rethrow", seen, 30);
    t.expect_str("destroyed once, after the outer catch", g_log.str(), "E+3 ~Guard ~E30");
}
POLY_TEST_MAIN(test)
