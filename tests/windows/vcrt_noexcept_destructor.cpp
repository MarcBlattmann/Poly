// poly-cxx: runtime=vcruntime140
//
// __CxxFrameHandler3 (specs/windows/msvcrt.dll/__CxxFrameHandler3.yaml, errors `clang_noexcept` and
// `terminate_destructor`): a destructor is `noexcept`, so the compiler gives it a terminate scope
// whose unwind action calls `__std_terminate`. An exception that leaves it while another one is
// being unwound makes the runtime end the unwind at the call of the destructor, but the frames
// below that call are unwound first: the terminate action of the destructor's scope runs, so
// `terminate()` comes from the program (`poly_std_terminate_calls` is 1), and nothing above the
// call is unwound (`~D` of the function does not run, no catch clause is entered). The expected
// values are the sheet's; Windows' own results are to be recorded by a spec agent.
#include "poly_wtest.h"

static poly::Log g_log;

static void on_terminate() {
    g_log.add("terminate");
    poly::g_test.expect_str("order: the destructor's own scope, then terminate", g_log.str(), "~Bad terminate");
#ifndef POLY_CXX_RUNTIME_VCRUNTIME140
    // (With vcruntime140.dll the compiler's call goes to that DLL's own __std_terminate, not counted here.)
    poly::g_test.expect("terminate came from the compiler's terminate scope", poly_std_terminate_calls, 1);
#endif
    poly::finish_and_exit();
}

struct Bad {
    // Implicitly noexcept: the compiler adds a terminate scope around the body.
    ~Bad() {
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
