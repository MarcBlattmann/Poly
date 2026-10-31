// poly-cxx: cflags=-fasync-exceptions
// __CxxFrameHandler3 with EH flags 0 and catch(...) clauses without the
// "C++ only" adjective (0x40), which is what clang -fasync-exceptions and
// /EHa emit: catch(...) also takes a non-C++ exception (RaiseException with
// another code); the frames between are unwound with their destructors
// first; a typed clause never takes it. The outer __except is not asked.
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
    } catch (int) {
        g_log.add("typed");
        return 3;
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
    t.expect("catch(...) took it", r, 1);
    t.expect_str("inner frame unwound first, then the catch block", g_log.str(), "~inner catch-all ~frame");
    g_log.clear();

    r = seh_frame(1);
    t.expect("a C++ int goes to the typed clause", r, 3);
    t.expect_str("its destructors run", g_log.str(), "typed ~frame");
}
POLY_TEST_MAIN(test)
