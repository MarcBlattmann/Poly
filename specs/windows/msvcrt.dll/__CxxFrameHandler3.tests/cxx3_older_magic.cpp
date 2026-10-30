// poly-cxx: magic=0x19930521
// __CxxFrameHandler3 also reads the older FuncInfo layouts: with magic
// 0x19930521 (no EH flags field; the flags word clang leaves after it is
// not read) catches, copies and unwinds work as with 0x19930522, and a
// non-C++ exception taken by an outer __except unwinds the C++ frames with
// their destructors (the synchronous-only flag does not exist for this
// layout); catch(...) with adjective 0x40 still does not take it.
#include "poly_wtest.h"

static poly::Log g_log;

struct D {
    const char* n;
    D(const char* n) : n(n) {}
    ~D() { g_log.add(n); }
};
struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

__declspec(noinline) static void throw_a(int v) {
    D d("~thrower");
    throw A(v);
}
__declspec(noinline) static void raise_seh() {
    D d("~inner");
    RaiseException(0xE0000003u, 0, 0, 0);
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
static int filter(unsigned code) { return code == 0xE0000003u; }
__declspec(noinline) static int seh_frame() {
    int r = 0;
    __try {
        r = cxx_frame();
    } __except (filter(__exception_code())) {
        g_log.add("except");
        r = 2;
    }
    return r;
}

static void test(poly::Test& t) {
    try {
        throw_a(1);
    } catch (A a) {
        g_log.add_num("catch", a.v);
    }
    t.expect_str("catch by value", g_log.str(), "A+1 Acopy101 ~thrower catch101 ~A101 ~A1");
    g_log.clear();

    t.expect("the __except took it", seh_frame(), 2);
    t.expect_str("non-C++ exception: destructors run, catch(...) passes", g_log.str(), "~inner ~frame except");
}
POLY_TEST_MAIN(test)
