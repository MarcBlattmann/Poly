// __CxxFrameHandler3 with FuncInfo EH flags bit 0 set (what clang and /EHs
// emit: synchronous exceptions only): a non-C++ exception (RaiseException
// with another code) is not seen by the C++ frames at all. catch(...) does
// not take it, and when an outer __except takes it, the C++ frames between
// are unwound without running their destructors. C++ exceptions keep
// working in the same functions afterwards.
#include "poly_wtest.h"

static poly::Log g_log;

struct D {
    const char* n;
    D(const char* n) : n(n) {}
    ~D() { g_log.add(n); }
};

__declspec(noinline) static void raise_seh() {
    D d("~inner");
    RaiseException(0xE0000001u, 0, 0, 0);
    g_log.add("returned");
}

__declspec(noinline) static int cxx_frame(int cxx) {
    D d("~frame");
    try {
        if (cxx) throw 9;
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

__declspec(noinline) static int seh_frame(int cxx) {
    int r = -1;
    __try {
        r = cxx_frame(cxx);
    } __except (filter(__exception_code())) {
        g_log.add("except");
        r = 2;
    }
    return r;
}

static void test(poly::Test& t) {
    int r = seh_frame(0);
    t.expect("the __except block took it", r, 2);
    t.expect_str("catch(...) passed, no destructors ran", g_log.str(), "filter except");
    g_log.clear();

    r = seh_frame(1);
    t.expect("a C++ exception afterwards: catch(...) takes it", r, 1);
    t.expect_str("its destructors run", g_log.str(), "catch-all ~frame");
}
POLY_TEST_MAIN(test)
