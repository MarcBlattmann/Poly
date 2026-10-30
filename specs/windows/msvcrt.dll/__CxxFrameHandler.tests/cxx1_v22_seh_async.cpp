// poly-cxx: handler=__CxxFrameHandler cflags=-fasync-exceptions
// __CxxFrameHandler, magic 0x19930522, EH flags 0 and catch(...) without
// adjective 0x40 (/EHa): catch(...) takes a non-C++ exception, after the
// inner frame was unwound.
#include "poly_wtest.h"

static poly::Log g_log;

struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

struct D {
    const char* n;
    D(const char* n) : n(n) {}
    ~D() { g_log.add(n); }
};
__declspec(noinline) static void raise_seh() {
    D d("~inner");
    RaiseException(0xE0000001u, 0, 0, 0);
}
__declspec(noinline) static int cxx_frame() {
    D d("~frame");
    try {
        raise_seh();
    } catch (...) {
        g_log.add("catch-all");
        return 1;
    }
    return 0;
}
static int filter(unsigned code) {
    g_log.add("filter");
    return code == 0xE0000001u ? POLY_EXCEPTION_EXECUTE_HANDLER : POLY_EXCEPTION_CONTINUE_SEARCH;
}
__declspec(noinline) static int seh_frame() {
    int r = -1;
    __try {
        r = cxx_frame();
    } __except (filter(__exception_code())) {
        g_log.add("except");
        r = 2;
    }
    return r;
}
static void test(poly::Test& t) {
    int r = seh_frame();
    t.expect("who took it", r, 1);
    t.expect_str("order", g_log.str(), "~inner catch-all ~frame");
    g_log.clear();
    int c = 0;
    try {
        throw A(9);
    } catch (...) {
        c = 1;
    }
    t.expect("C++ exceptions still go to catch(...)", c, 1);
}
POLY_TEST_MAIN(test)
