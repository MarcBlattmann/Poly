// __CxxFrameHandler3 and structured exception handling in one call chain.
// A C++ exception is an SEH exception (code 0xE06D7363): an __except filter
// between the throw and the C++ catch is asked during the search, before
// any destructor runs; when it returns EXCEPTION_CONTINUE_SEARCH the C++
// catch further out takes the exception, and a __finally block between
// runs during the unwind, in frame order with the destructors. When an
// __except takes a C++ exception, the exception object is destroyed first
// (see _CxxThrowException), then the C++ frames between are unwound with
// their destructors (a C++ exception is never filtered out by the EH
// flags).
#include "poly_wtest.h"

static poly::Log g_log;

struct D {
    const char* n;
    D(const char* n) : n(n) {}
    ~D() { g_log.add(n); }
};
struct Obj {
    int v;
    Obj(int x) : v(x) { g_log.add_num("Obj+", v); }
    Obj(const Obj& o) : v(o.v) { g_log.add("Objcopy"); }
    ~Obj() { g_log.add_num("~Obj", v); }
};

__declspec(noinline) static void thrower(int v) {
    D d("~thrower");
    throw Obj(v);
}

static int filter_continue(unsigned code) {
    g_log.add(code == 0xE06D7363u ? "filter-cxx" : "filter-other");
    return POLY_EXCEPTION_CONTINUE_SEARCH;
}
static int filter_take(unsigned code) {
    g_log.add(code == 0xE06D7363u ? "take-cxx" : "take-other");
    return POLY_EXCEPTION_EXECUTE_HANDLER;
}

__declspec(noinline) static void with_filter(int v) {
    __try {
        thrower(v);
    } __except (filter_continue(__exception_code())) {
        g_log.add("not here");
    }
}
__declspec(noinline) static void cxx_middle(int v) {
    D d("~middle");
    with_filter(v);
}
__declspec(noinline) static void with_finally(int v) {
    __try {
        cxx_middle(v);
    } __finally {
        g_log.add("finally");
    }
}

__declspec(noinline) static int seh_takes(int v) {
    int r = 0;
    __try {
        cxx_middle(v);
    } __except (filter_take(__exception_code())) {
        g_log.add("except");
        r = 1;
    }
    return r;
}

static void test(poly::Test& t) {
    try {
        with_finally(1);
    } catch (Obj& o) {
        g_log.add_num("catch", o.v);
    }
    t.expect_str("filter first; then destructors and __finally in frame order", g_log.str(),
                 "Obj+1 filter-cxx ~thrower ~middle finally catch1 ~Obj1");
    g_log.clear();

    int r = seh_takes(2);
    t.expect("the __except block took the C++ exception", r, 1);
    t.expect_str("object destroyed first, then the C++ frames with their destructors", g_log.str(),
                 "Obj+2 filter-cxx take-cxx ~Obj2 ~thrower ~middle except");
}
POLY_TEST_MAIN(test)
