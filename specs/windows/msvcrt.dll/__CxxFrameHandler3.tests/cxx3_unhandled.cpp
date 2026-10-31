// __CxxFrameHandler3: a C++ exception that no clause matches is an
// unhandled exception like any other: the search ends at the top-level
// filter (SetUnhandledExceptionFilter) with code 0xE06D7363; no unwind took
// place (no destructor ran) and terminate() is not called.
#include "poly_wtest.h"

static poly::Log g_log;

static void on_terminate() {
    g_log.add("terminate");
    poly::g_test.expect_str("terminate was not expected", g_log.str(), "");
    poly::finish_and_exit();
}

static long __stdcall top_filter(PolyExceptionPointers* p) {
    g_log.add("top");
    poly::g_test.expect_hex("code", p->ExceptionRecord->ExceptionCode, 0xE06D7363u);
    poly::g_test.expect("parameters", p->ExceptionRecord->NumberParameters, 4);
    poly::g_test.expect_hex("magic", p->ExceptionRecord->ExceptionInformation[0], 0x19930520u);
    poly::g_test.expect_str("no destructor ran before the top-level filter", g_log.str(), "top");
    poly::finish_and_exit();
}

struct D {
    const char* n;
    D(const char* n) : n(n) {}
    ~D() { g_log.add(n); }
};

__declspec(noinline) static void inner() {
    D d("~inner");
    throw 1;
}
__declspec(noinline) static void middle() {
    D d("~middle");
    try {
        inner();
    } catch (long) {
        g_log.add("wrong clause");
    } catch (char*) {
        g_log.add("wrong clause");
    }
}

static void test(poly::Test& t) {
    set_terminate(on_terminate);
    SetUnhandledExceptionFilter((void*)top_filter);
    middle();
    t.expect_str("not reached", g_log.str(), "the top-level filter before this");
}
POLY_TEST_MAIN(test)
