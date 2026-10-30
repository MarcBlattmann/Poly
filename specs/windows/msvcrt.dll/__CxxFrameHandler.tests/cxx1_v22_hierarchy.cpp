// poly-cxx: handler=__CxxFrameHandler
// __CxxFrameHandler, magic 0x19930522: the same type matching as with the
// oldest layout.
#include "poly_wtest.h"

static poly::Log g_log;

struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

struct Base {
    int b;
    Base(int x) : b(x) {}
    virtual ~Base() {}
    virtual int kind() const { return 1; }
};
struct Other {
    long long o = 5;
};
struct Derived : Other, Base {
    Derived(int x) : Base(x) {}
    int kind() const override { return 2; }
};

static const void* g_made;
__declspec(noinline) static void throw_derived(int v) {
    Derived d(v);
    g_made = &d;
    throw d;
}
__declspec(noinline) static void throw_ptr(Derived* p) { throw p; }
__declspec(noinline) static void throw_cptr(const int* p) { throw p; }

static void test(poly::Test& t) {
    int r = 0;
    try {
        throw_derived(3);
    } catch (Base& b) {
        r = b.kind() * 10 + b.b;
    }
    t.expect("Derived caught as its second base, by reference", r, 23);

    r = 0;
    try {
        throw_derived(4);
    } catch (Base b) {
        r = b.kind() * 10 + b.b;
    }
    t.expect("as a base by value (sliced)", r, 14);

    r = 0;
    try {
        throw_derived(5);
    } catch (Other& o) {
        r = (int)o.o;
    }
    t.expect("as its first base", r, 5);

    Derived d(6);
    r = 0;
    try {
        throw_ptr(&d);
    } catch (Base* b) {
        r = b == static_cast<Base*>(&d) ? b->b : -1;
    }
    t.expect("Derived* as Base*, adjusted", r, 6);

    r = 0;
    try {
        throw_ptr(&d);
    } catch (void* v) {
        r = v == (void*)&d ? 7 : -7;
    }
    t.expect("Derived* as void*", r, 7);

    int x = 8;
    r = 0;
    try {
        throw_cptr(&x);
    } catch (int*) {
        r = 1;
    } catch (const int* p) {
        r = *p;
    }
    t.expect("const int* not as int*", r, 8);
}
POLY_TEST_MAIN(test)
