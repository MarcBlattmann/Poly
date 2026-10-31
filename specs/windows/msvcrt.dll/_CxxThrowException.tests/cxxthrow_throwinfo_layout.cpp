// The ThrowInfo a throw passes to _CxxThrowException, read through the
// raised record (every field below is an offset from the image base in
// parameter 3): ThrowInfo { attributes, destructor, forward compat,
// catchable type array }, CatchableTypeArray { count, offsets... },
// CatchableType { properties, type descriptor, mdisp, pdisp, vdisp, size,
// copy function }, TypeDescriptor { vftable pointer, spare pointer,
// decorated name }. The array lists the thrown type first (for a class:
// then its unambiguous public bases; for a pointer to an object: then
// void*); attributes bit 0 is const, bit 1 volatile (of a pointer's
// pointee); properties bit 0 is "simple type" (scalar or pointer).
#include "poly_wtest.h"

static PolyExceptionRecord g_rec;

static int capture(void* info) {
    g_rec = *((PolyExceptionPointers*)info)->ExceptionRecord;
    return POLY_EXCEPTION_EXECUTE_HANDLER;
}

struct Base {
    int b = 1;
    virtual ~Base() {}
};
struct Derived : Base {
    long long d = 2;
};
struct NoDtor {
    char c[12];
};

__declspec(noinline) static void throw_int() { throw 5; }
__declspec(noinline) static void throw_derived() { throw Derived(); }
__declspec(noinline) static void throw_const_ptr() {
    static const int x = 3;
    throw &x;
}
__declspec(noinline) static void throw_volatile_ptr() {
    static volatile int y = 4;
    throw &y;
}
__declspec(noinline) static void throw_nodtor() { throw NoDtor{}; }

static void seh(void (*f)()) {
    __try {
        f();
    } __except (capture(__exception_info())) {
    }
}

static void dump(poly::Test& t, const char* what) {
    t.line(what);
    poly_u64 b = g_rec.ExceptionInformation[3];
    const poly_u32* ti = (const poly_u32*)g_rec.ExceptionInformation[2];
    t.value_hex("attributes", ti[0]);
    t.value("has destructor", ti[1] != 0);
    t.value_hex("forward compat", ti[2]);
    const poly_u32* cta = (const poly_u32*)(b + ti[3]);
    t.value("catchable types", cta[0]);
    for (poly_u32 i = 0; i < cta[0]; i++) {
        const int* ct = (const int*)(b + cta[1 + i]);
        const char* td = (const char*)(b + (poly_u32)ct[1]);
        t.line(td + 16);
        t.value_hex("  properties", (poly_u32)ct[0]);
        t.value("  mdisp", ct[2]);
        t.value("  pdisp", ct[3]);
        t.value("  vdisp", ct[4]);
        t.value("  size", ct[5]);
        t.value("  has copy function", ct[6] != 0);
        t.check("  vftable pointer is type_info's", *(const void* const*)td == (const void*)poly_type_info_vftable, 0);
    }
}

static void test(poly::Test& t) {
    seh(throw_int);
    dump(t, "# int");
    seh(throw_derived);
    dump(t, "# Derived");
    seh(throw_const_ptr);
    dump(t, "# const int*");
    seh(throw_volatile_ptr);
    dump(t, "# volatile int*");
    seh(throw_nodtor);
    dump(t, "# NoDtor");
}
POLY_TEST_MAIN(test)
