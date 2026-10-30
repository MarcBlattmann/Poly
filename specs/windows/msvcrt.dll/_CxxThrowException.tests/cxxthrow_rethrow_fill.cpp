// A rethrow record (parameters 1 to 3 zero) is completed by the C++ frame
// handler of the catch block it leaves: once the search has passed that
// frame, every filter further out sees the record of the exception being
// rethrown (the original object, ThrowInfo and image base; code, flags
// 0x81 and magic unchanged), whether `throw;` is in the catch block itself
// or in a function it calls, and also past more C++ frames with clauses
// that do not match. (A filter between the rethrow and that frame sees the
// zeros: cxxthrow_rethrow_record.)
#include "poly_wtest.h"

static PolyExceptionRecord g_rec;
static const void* g_obj;

static int capture(void* info) {
    g_rec = *((PolyExceptionPointers*)info)->ExceptionRecord;
    return POLY_EXCEPTION_EXECUTE_HANDLER;
}

struct Obj {
    int v;
    Obj(int x) : v(x) { g_obj = this; }
    Obj(const Obj& o) : v(o.v) {}
    ~Obj() {}
};

__declspec(noinline) static void throw_obj(int v) { throw Obj(v); }

// The decorated name of the first catchable type of a record's ThrowInfo.
static const char* first_type_name(const PolyExceptionRecord& r) {
    poly_u64 base = r.ExceptionInformation[3];
    const poly_u32* ti = (const poly_u32*)r.ExceptionInformation[2];
    if (!ti || !base) return "";
    const poly_u32* cta = (const poly_u32*)(base + ti[3]);
    const poly_u32* ct = (const poly_u32*)(base + cta[1]);
    return (const char*)(base + ct[1]) + 16;
}

__declspec(noinline) static void abandon() {
    try {
        throw_obj(2);
    } catch (Obj&) {
        throw;
    }
}
__declspec(noinline) static void helper() { throw; }
__declspec(noinline) static void abandon_via_helper() {
    try {
        throw_obj(3);
    } catch (Obj&) {
        helper();
    }
}
__declspec(noinline) static void passthrough() {
    try {
        abandon();
    } catch (int) {
    } catch (long) {
    }
}

static void seh(void (*f)()) {
    __try {
        f();
    } __except (capture(__exception_info())) {
    }
}

static void check(poly::Test& t, const char* what) {
    t.line(what);
    t.expect_hex("code", g_rec.ExceptionCode, 0xE06D7363u);
    t.expect_hex("flags", g_rec.ExceptionFlags, 0x81u);
    t.expect("parameters", g_rec.NumberParameters, 4);
    t.expect_hex("magic", g_rec.ExceptionInformation[0], 0x19930520u);
    t.check("the original object", (const void*)g_rec.ExceptionInformation[1] == g_obj, 0);
    t.check("the image base", g_rec.ExceptionInformation[3] == (poly_u64)GetModuleHandleW(0), 0);
    t.expect_str("the original throw info", first_type_name(g_rec), ".?AUObj@@");
}

static void test(poly::Test& t) {
    seh(abandon);
    check(t, "# throw; in the catch block");
    seh(abandon_via_helper);
    check(t, "# throw; in a function the catch block calls");
    seh(passthrough);
    check(t, "# past another C++ frame whose clauses do not match");
}
POLY_TEST_MAIN(test)
