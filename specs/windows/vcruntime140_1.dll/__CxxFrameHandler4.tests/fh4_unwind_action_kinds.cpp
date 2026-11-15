// poly-cxx: compiler=msvc
// __CxxFrameHandler4: the kinds of cleanup the unwind runs, each in its own function so each has
// its own table entry kind: a destructor of a local (object in the frame), of a parameter passed
// by value (the object is reached through a pointer saved in the frame), of an object with a
// virtual base (the vbase destructor), of a named return value (a cleanup funclet), of an
// over-aligned local (a cleanup funclet: its address is computed from the frame), and of
// temporaries inside one full-expression; plus a class-specific placement delete for a
// constructor that throws in a placement new expression.
#include "poly_wtest.h"

static poly::Log g_log;

struct D {
    int n;
    D(int n) : n(n) { g_log.add_num("D+", n); }
    D(const D& o) : n(o.n + 100) { g_log.add_num("Dcopy", n); }
    ~D() { g_log.add_num("~D", n); }
};

struct alignas(64) Aligned {
    int n;
    Aligned(int n) : n(n) { g_log.add_num("A+", n); }
    ~Aligned() { g_log.add_num("~A", n); }
};

struct V {
    virtual ~V() { g_log.add("~V"); }
};
struct W : virtual V {
    W() { g_log.add("W+"); }
    ~W() { g_log.add("~W"); }
};

struct Bomb {
    Bomb(bool boom) {
        g_log.add("Bomb+");
        if (boom) throw 7;
    }
    ~Bomb() { g_log.add("~Bomb"); }
    static void* operator new(decltype(sizeof 0), void* where) noexcept { return where; }
    static void operator delete(void*, void*) noexcept { g_log.add("placement delete"); }
    static void operator delete(void*) noexcept { g_log.add("delete"); }
};

__declspec(noinline) static void thrower() { throw 1; }
__declspec(noinline) static int make(const D&, const D&) { thrower(); return 0; }

__declspec(noinline) static void by_value_param(D d) { thrower(); }
__declspec(noinline) static D named_return() {
    D r(5);
    thrower();
    return r;
}
__declspec(noinline) static void aligned_local() {
    Aligned a(6);
    thrower();
}
__declspec(noinline) static void virtual_base() {
    W w;
    thrower();
}
__declspec(noinline) static void temporaries() { make(D(7), D(8)); }
__declspec(noinline) static void placement(void* storage) {
    Bomb* b = new (storage) Bomb(true);
    (void)b;
}

static void expect_unwind(poly::Test& t, const char* what, void (*fn)(), const char* want) {
    g_log.clear();
    try {
        fn();
    } catch (int) {
        g_log.add("caught");
    }
    t.expect_str(what, g_log.str(), want);
}

static void test(poly::Test& t) {
    expect_unwind(t, "local object", [] { D d(1); thrower(); }, "D+1 ~D1 caught");
    expect_unwind(t, "parameter passed by value (the caller made the copy)", [] { by_value_param(D(2)); },
                  "D+2 ~D2 caught");
    expect_unwind(t, "named return value", [] { named_return(); }, "D+5 ~D5 caught");
    expect_unwind(t, "over-aligned local", aligned_local, "A+6 ~A6 caught");
    expect_unwind(t, "object with a virtual base", virtual_base, "W+ ~W ~V caught");
    expect_unwind(t, "temporaries of one full-expression", temporaries, "D+8 D+7 ~D7 ~D8 caught");
    alignas(16) char storage[64];
    g_log.clear();
    try {
        placement(storage);
    } catch (int) {
        g_log.add("caught");
    }
    t.expect_str("placement new whose constructor throws calls the matching placement delete", g_log.str(),
                 "Bomb+ placement delete caught");
}
POLY_TEST_MAIN(test)
