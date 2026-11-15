// poly-cxx: runtime=vcruntime140
//
// vcruntime140.dll (ADR 0199): the per-thread exception state a program of
// Visual C++ 2015 or later reads through the runtime. The expected values are
// the C++ standard's (C++17 [uncaught.exceptions]: an exception counts as
// uncaught from its throw until its handler is entered, so a destructor run
// by the unwind sees 1 and the catch block 0), and the meaning of the
// exported names: `__current_exception()` points at the EXCEPTION_RECORD of
// the exception the innermost running catch block handles (null outside one),
// whose second parameter is the exception object a by-reference catch binds
// to (`_CxxThrowException.yaml`). Implementer's tests: Windows' own results
// are to be recorded by a spec agent (ADR 0199).
#include "poly_wtest.h"

extern "C" {
__declspec(dllimport) void** __current_exception();
__declspec(dllimport) int __uncaught_exceptions();
__declspec(dllimport) bool __uncaught_exception();
}

struct Probe {
    int* seen;
    ~Probe() { *seen = __uncaught_exceptions(); }
};

__declspec(noinline) static void throw_int(int* seen, int v) {
    Probe p{seen};
    throw v;
}

static PolyExceptionRecord* current() { return (PolyExceptionRecord*)*__current_exception(); }

static void test(poly::Test& t) {
    t.expect("none uncaught before", __uncaught_exceptions(), 0);
    t.check("no current exception before", current() == 0, 0);
    int during = -1, in_catch = -1, inner_count = -1;
    poly_u64 outer_object = 0, inner_object = 0, outer_again = 0, outer_seen = 0, inner_seen = 0;
    bool any = true;
    try {
        throw_int(&during, 1);
    } catch (int& e) {
        in_catch = __uncaught_exceptions();
        any = __uncaught_exception();
        PolyExceptionRecord* r = current();
        t.check("a current exception in the catch block", r != 0, 0);
        if (r) {
            t.expect_hex("its code", r->ExceptionCode, 0xE06D7363u);
            outer_object = r->ExceptionInformation[1];
        }
        outer_seen = (poly_u64)&e;
        try {
            int ignored = 0;
            throw_int(&ignored, 2);
        } catch (int& f) {
            inner_count = __uncaught_exceptions();
            PolyExceptionRecord* r2 = current();
            inner_object = r2 ? r2->ExceptionInformation[1] : 0;
            inner_seen = (poly_u64)&f;
        }
        PolyExceptionRecord* r3 = current();
        outer_again = r3 ? r3->ExceptionInformation[1] : 0;
    }
    t.expect("one uncaught while the unwind runs a destructor", during, 1);
    t.expect("none uncaught in the catch block", in_catch, 0);
    t.check("__uncaught_exception() is false in the catch block", !any, any);
    t.check("the record names the object the catch binds to", outer_object == outer_seen && outer_seen != 0, 0);
    t.expect("none uncaught in the nested catch block", inner_count, 0);
    t.check("the nested catch block's record is its own", inner_object == inner_seen && inner_seen != 0, 0);
    t.check("after it, the outer exception is current again", outer_again == outer_seen, 0);
    t.expect("none uncaught after", __uncaught_exceptions(), 0);
    t.check("no current exception after", current() == 0, 0);
}
POLY_TEST_MAIN(test)
