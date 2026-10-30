// __CxxFrameHandler3: during an unwind every fully constructed local
// object of every frame between the throw and the catch is destroyed, the
// innermost frame first and, within a frame, in reverse order of
// construction; objects of scopes that were left already, and objects not
// yet constructed, are not. A constructor that throws destroys the
// members and bases already constructed, not the object itself. Array
// elements already constructed are destroyed in reverse order.
#include "poly_wtest.h"

static poly::Log g_log;

struct N {
    int id;
    N(int i) : id(i) { g_log.add_num("+", id); }
    ~N() { g_log.add_num("-", id); }
};

struct Fragile {
    N first;
    N second;
    Fragile(int fail) : first(10), second(11) {
        if (fail) throw 1;
    }
    ~Fragile() { g_log.add("~Fragile"); }
};

struct Elem {
    static int made;
    int id;
    Elem() : id(made++) {
        if (id == 3) throw 2;
        g_log.add_num("e+", id);
    }
    ~Elem() { g_log.add_num("e-", id); }
};
int Elem::made = 0;

__declspec(noinline) static void level3() {
    N a(31);
    {
        N gone(32);
    }
    N b(33);
    throw 3;
    N never(34);
}
__declspec(noinline) static void level2() {
    N a(21);
    level3();
}
__declspec(noinline) static void level1() {
    N a(11);
    N b(12);
    level2();
}

__declspec(noinline) static void construct_fragile() {
    N outer(5);
    Fragile f(1);
}

__declspec(noinline) static void make_array() {
    Elem arr[5];
    g_log.add("array done");
}

static void test(poly::Test& t) {
    try {
        level1();
    } catch (int x) {
        g_log.add_num("catch", x);
    }
    t.expect_str("innermost frame first, reverse construction order", g_log.str(),
                 "+11 +12 +21 +31 +32 -32 +33 -33 -31 -21 -12 -11 catch3");
    g_log.clear();

    try {
        construct_fragile();
    } catch (int x) {
        g_log.add_num("catch", x);
    }
    t.expect_str("a throwing constructor destroys its members, not itself", g_log.str(),
                 "+5 +10 +11 -11 -10 -5 catch1");
    g_log.clear();

    Elem::made = 0;
    try {
        make_array();
    } catch (int x) {
        g_log.add_num("catch", x);
    }
    t.expect_str("constructed array elements in reverse", g_log.str(), "e+0 e+1 e+2 e-2 e-1 e-0 catch2");
}
POLY_TEST_MAIN(test)
