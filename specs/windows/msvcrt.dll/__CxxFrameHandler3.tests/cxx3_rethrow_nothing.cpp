// __CxxFrameHandler3: `throw;` while no exception is being handled raises
// the rethrow record (0xE06D7363, parameters 1 to 3 zero) all the same; no
// clause takes it, not even catch(...), so it ends at the top-level filter
// as an unhandled exception; terminate() is not called.
#include "poly_wtest.h"

static poly::Log g_log;

static void on_terminate() {
    g_log.add("terminate");
    poly::g_test.expect_str("terminate was not expected", g_log.str(), "");
    poly::finish_and_exit();
}

static long __stdcall top_filter(PolyExceptionPointers* p) {
    PolyExceptionRecord* r = p->ExceptionRecord;
    g_log.add("top");
    poly::g_test.expect_hex("code", r->ExceptionCode, 0xE06D7363u);
    poly::g_test.expect("parameters", r->NumberParameters, 4);
    poly::g_test.expect_hex("magic", r->ExceptionInformation[0], 0x19930520u);
    poly::g_test.check("no object, no throw info, no image base",
                       r->ExceptionInformation[1] == 0 && r->ExceptionInformation[2] == 0 &&
                           r->ExceptionInformation[3] == 0,
                       0);
    poly::g_test.expect_str("catch(...) did not take it", g_log.str(), "top");
    poly::finish_and_exit();
}

__declspec(noinline) static void rethrow() { throw; }

static void test(poly::Test& t) {
    set_terminate(on_terminate);
    SetUnhandledExceptionFilter((void*)top_filter);
    try {
        rethrow();
    } catch (int) {
        g_log.add("int");
    } catch (...) {
        g_log.add("catch-all");
    }
    t.expect_str("not reached", g_log.str(), "the top-level filter before this");
}
POLY_TEST_MAIN(test)
