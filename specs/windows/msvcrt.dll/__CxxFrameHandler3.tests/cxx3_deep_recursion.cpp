// __CxxFrameHandler3: an exception thrown 200 frames deep, each frame with
// an object to destroy, unwinds every frame (each destructor once, deepest
// first) to the catch at the top; a second throw afterwards works the same.
#include "poly_wtest.h"

static int g_made, g_destroyed, g_last_destroyed, g_order_ok;

struct Counter {
    int depth;
    Counter(int d) : depth(d) { g_made++; }
    ~Counter() {
        if (g_last_destroyed != 0 && depth != g_last_destroyed - 1) g_order_ok = 0;
        g_last_destroyed = depth;
        g_destroyed++;
    }
};

__declspec(noinline) static int recurse(int depth, int limit) {
    Counter c(depth);
    if (depth == limit) throw depth;
    return recurse(depth + 1, limit) + 1;
}

static void run(poly::Test& t, const char* what, int limit) {
    g_made = g_destroyed = g_last_destroyed = 0;
    g_order_ok = 1;
    int caught = 0;
    try {
        recurse(1, limit);
    } catch (int d) {
        caught = d;
    }
    t.expect(what, caught, limit);
    t.expect("every frame's object was destroyed", g_destroyed, g_made);
    t.expect("as many as frames", g_made, limit);
    t.expect("deepest first", g_order_ok, 1);
}

static void test(poly::Test& t) {
    run(t, "caught from depth 200", 200);
    run(t, "caught from depth 37", 37);
}
POLY_TEST_MAIN(test)
