// __CxxFrameHandler3: after a catch block finishes, the function goes on
// after its try statement with a consistent state: objects constructed
// after the catch are destroyed by a later throw (and objects destroyed
// before are not destroyed twice); a function that catches in a loop
// keeps working for a thousand iterations; locals changed in a catch block
// keep their values.
#include "poly_wtest.h"

static poly::Log g_log;

struct N {
    int id;
    N(int i) : id(i) { g_log.add_num("+", id); }
    ~N() { g_log.add_num("-", id); }
};

__declspec(noinline) static void throw_int(int v) { throw v; }

__declspec(noinline) static void after_catch() {
    N a(1);
    try {
        N b(2);
        throw_int(10);
    } catch (int) {
        g_log.add("c");
    }
    N c(3);
    try {
        N d(4);
        throw_int(20);
    } catch (int) {
        g_log.add("c");
    }
    N e(5);
    throw_int(30);
}

static void test(poly::Test& t) {
    try {
        after_catch();
    } catch (int v) {
        g_log.add_num("outer", v);
    }
    t.expect_str("states after each catch", g_log.str(), "+1 +2 -2 c +3 +4 -4 c +5 -5 -3 -1 outer30");
    g_log.clear();

    int ints = 0, objs = 0, sum = 0;
    for (int i = 0; i < 1000; i++) {
        try {
            N guard(-1);
            if (i % 3 == 0) throw_int(i);
            if (i % 3 == 1) throw N(i);
            sum += 1;
        } catch (int v) {
            ints++;
            sum += v % 7;
        } catch (N& n) {
            objs++;
            sum += n.id % 5;
        }
        if (i == 2) g_log.clear();
    }
    t.expect("ints", ints, 334);
    t.expect("objects", objs, 333);
    int want = 0;
    for (int i = 0; i < 1000; i++) want += i % 3 == 0 ? i % 7 : i % 3 == 1 ? i % 5 : 1;
    t.expect("locals changed in catch blocks", sum, want);
}
POLY_TEST_MAIN(test)
