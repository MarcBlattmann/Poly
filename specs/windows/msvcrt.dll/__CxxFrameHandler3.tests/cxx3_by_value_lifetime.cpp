// __CxxFrameHandler3: a by-value catch object is made with the catchable
// type's copy constructor before the frames between the throw and the
// catch are unwound (before their destructors run); it is destroyed when
// the catch block ends, and then the exception object itself is destroyed
// (the ThrowInfo's destructor), once. A clause without a catch object (an
// unnamed parameter: catch object offset 0) copies nothing. A class
// without a user copy constructor is copied bit for bit (size bytes); a
// large object too.
#include "poly_wtest.h"

static poly::Log g_log;

struct A {
    int v;
    A(int x) : v(x) { g_log.add_num("A+", v); }
    A(const A& o) : v(o.v + 100) { g_log.add_num("Acopy", v); }
    ~A() { g_log.add_num("~A", v); }
};

struct Plain {
    int a, b, c;
};

struct Big {
    unsigned char bytes[256];
    int tag;
};

__declspec(noinline) static void throw_a(int k) {
    A local(k);
    throw A(k + 1);
}
__declspec(noinline) static void throw_plain() {
    A guard(50);
    throw Plain{1, 2, 3};
}
__declspec(noinline) static void throw_big() {
    Big b;
    for (int i = 0; i < 256; i++) b.bytes[i] = (unsigned char)(i * 7);
    b.tag = 0x1234;
    throw b;
}

static void test(poly::Test& t) {
    try {
        throw_a(1);
    } catch (A a) {
        g_log.add_num("catch", a.v);
    }
    g_log.add("after");
    t.expect_str("copy before unwind, param then object destroyed", g_log.str(),
                 "A+1 A+2 Acopy102 ~A1 catch102 ~A102 ~A2 after");
    g_log.clear();

    try {
        throw_a(10);
    } catch (const A a) {
        g_log.add_num("catch", a.v);
    }
    t.expect_str("const by value is the same", g_log.str(), "A+10 A+11 Acopy111 ~A10 catch111 ~A111 ~A11");
    g_log.clear();

    try {
        throw_a(20);
    } catch (...) {
        g_log.add("catch-all");
    }
    t.expect_str("catch(...) copies nothing, destroys the object at its end", g_log.str(),
                 "A+20 A+21 ~A20 catch-all ~A21");
    g_log.clear();

    try {
        throw_a(30);
    } catch (A) {
        g_log.add("unnamed");
    }
    t.expect_str("an unnamed by-value clause (catch object offset 0) copies nothing", g_log.str(),
                 "A+30 A+31 ~A30 unnamed ~A31");
    g_log.clear();

    int sum = 0;
    try {
        throw_plain();
    } catch (Plain p) {
        sum = p.a * 100 + p.b * 10 + p.c;
    }
    t.expect("trivially copyable class by value", sum, 123);
    t.expect_str("its frame was unwound", g_log.str(), "A+50 ~A50");
    g_log.clear();

    int ok = 0;
    try {
        throw_big();
    } catch (Big b) {
        ok = b.tag == 0x1234;
        for (int i = 0; i < 256; i++)
            if (b.bytes[i] != (unsigned char)(i * 7)) ok = 0;
    }
    t.expect("256-byte object by value", ok, 1);
}
POLY_TEST_MAIN(test)
