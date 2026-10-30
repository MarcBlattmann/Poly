// poly-cxx: handler=__CxxFrameHandler
// __CxxFrameHandler: a C++ exception no clause takes reaches the
// top-level filter without any unwind.
#include "poly_wtest.h"

static poly::Log g_log;

struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

static void on_terminate() {
    poly::g_test.line("terminate was not expected");
    poly::finish_and_exit();
}
static long __stdcall top_filter(PolyExceptionPointers* p) {
    g_log.add("top");
    poly::g_test.expect_hex("code", p->ExceptionRecord->ExceptionCode, 0xE06D7363u);
    poly::g_test.expect_str("no unwind before the top-level filter", g_log.str(), "A+1 top");
    poly::finish_and_exit();
}
__declspec(noinline) static void inner() {
    A a(1);
    try {
        throw 5;
    } catch (long) {
        g_log.add("wrong");
    }
}
static void test(poly::Test& t) {
    set_terminate(on_terminate);
    SetUnhandledExceptionFilter((void*)top_filter);
    inner();
    t.expect_str("not reached", g_log.str(), "the top-level filter before this");
}
POLY_TEST_MAIN(test)
