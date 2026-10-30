// `throw;` is _CxxThrowException(NULL, NULL): the record raised has code
// 0xE06D7363, flags 0x81, four parameters, the magic number 0x19930520 and
// zero for the object, the ThrowInfo and the image base, whether or not an
// exception is being handled. A filter that takes it ends the rethrow; the
// catch block it came from continues after the __except.
#include "poly_wtest.h"

static PolyExceptionRecord g_rec;
static int g_seen;

static int capture(void* info) {
    g_rec = *((PolyExceptionPointers*)info)->ExceptionRecord;
    g_seen++;
    return POLY_EXCEPTION_EXECUTE_HANDLER;
}

__declspec(noinline) static void rethrow() { throw; }

static int seh_rethrow() {
    __try {
        rethrow();
    } __except (capture(__exception_info())) {
        return 1;
    }
    return 0;
}

static void check(poly::Test& t, const char* what) {
    t.line(what);
    t.expect_hex("code", g_rec.ExceptionCode, 0xE06D7363u);
    t.expect_hex("flags", g_rec.ExceptionFlags, 0x81u);
    t.expect("parameters", g_rec.NumberParameters, 4);
    t.expect_hex("magic", g_rec.ExceptionInformation[0], 0x19930520u);
    t.check("object 0", g_rec.ExceptionInformation[1] == 0, 0);
    t.check("throw info 0", g_rec.ExceptionInformation[2] == 0, 0);
    t.check("image base 0", g_rec.ExceptionInformation[3] == 0, 0);
}

static void test(poly::Test& t) {
    int after = 0;
    try {
        throw 7;
    } catch (int v) {
        t.expect("filter took the rethrow", seh_rethrow(), 1);
        check(t, "# inside a catch block");
        after = v;
    }
    t.expect("the catch block went on", after, 7);

    t.expect("filter took the rethrow", seh_rethrow(), 1);
    check(t, "# no exception being handled");
    t.expect("filter calls", g_seen, 2);
}
POLY_TEST_MAIN(test)
