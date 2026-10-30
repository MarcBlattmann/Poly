// _CxxThrowException(object, throw_info) raises a noncontinuable SEH
// exception: code 0xE06D7363, flags 0x81 (EXCEPTION_NONCONTINUABLE and
// 0x80, which RaiseException itself adds on this Windows build: a plain
// RaiseException with flags 0 shows 0x80 too), four parameters: the magic
// number 0x19930520, the object's address (the throw does not copy it),
// the ThrowInfo's address and the base of the image that holds the
// ThrowInfo (its fields are offsets from it); no chained record.
#include "poly_wtest.h"

static PolyExceptionRecord g_rec;
static int g_seen;

static int g_first_int;
static int capture(void* info) {
    g_rec = *((PolyExceptionPointers*)info)->ExceptionRecord;
    // The object lives in the throwing frame: read it while that frame is
    // still there (during the search), not after the unwind.
    if (g_rec.NumberParameters == 4 && g_rec.ExceptionInformation[1])
        g_first_int = *(const int*)g_rec.ExceptionInformation[1];
    g_seen++;
    return POLY_EXCEPTION_EXECUTE_HANDLER;
}

static const void* g_object;
struct Thing {
    int v;
    Thing(int x) : v(x) { g_object = this; }
    Thing(const Thing& o) : v(o.v) { g_object = this; }
    ~Thing() {}
};

__declspec(noinline) static void throw_int() { throw 5; }
__declspec(noinline) static void throw_thing() { throw Thing(6); }
__declspec(noinline) static void raise_plain() { RaiseException(0xE0000002u, 0, 0, 0); }

static int seh(void (*f)()) {
    __try {
        f();
    } __except (capture(__exception_info())) {
        return 1;
    }
    return 0;
}

static void check_record(poly::Test& t, const char* what) {
    t.line(what);
    t.expect_hex("code", g_rec.ExceptionCode, 0xE06D7363u);
    t.expect_hex("flags", g_rec.ExceptionFlags, 0x81u);
    t.expect("parameters", g_rec.NumberParameters, 4);
    t.expect_hex("magic", g_rec.ExceptionInformation[0], 0x19930520u);
    t.check("no chained record", g_rec.ExceptionRecord == 0, 0);
    poly_u64 base = (poly_u64)GetModuleHandleW(0);
    t.check("image base of the throw info's image", g_rec.ExceptionInformation[3] == base, 0);
    poly_u64 ti = g_rec.ExceptionInformation[2];
    t.check("throw info inside that image", ti > base && ti - base < 0x100000, 0);
}

static void test(poly::Test& t) {
    t.expect("seen", seh(throw_int), 1);
    check_record(t, "# throw 5");
    t.check("object is a pointer", g_rec.ExceptionInformation[1] != 0, 0);
    t.expect("object holds the value", g_first_int, 5);

    t.expect("seen", seh(throw_thing), 1);
    check_record(t, "# throw Thing(6)");
    t.check("object is the thrown object itself", (const void*)g_rec.ExceptionInformation[1] == g_object, 0);

    t.expect("seen", seh(raise_plain), 1);
    t.expect_hex("RaiseException with flags 0 shows", g_rec.ExceptionFlags, 0x80u);
    t.expect("filter calls", g_seen, 3);
}
POLY_TEST_MAIN(test)
