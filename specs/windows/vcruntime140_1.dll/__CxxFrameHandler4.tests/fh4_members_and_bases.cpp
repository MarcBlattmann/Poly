// poly-cxx: compiler=msvc
// __CxxFrameHandler4: a constructor that throws destroys the members and bases already
// constructed, in reverse order, and not the object itself; a function-try-block of a
// constructor catches an exception from a member initialiser and rethrows it.
#include "poly_wtest.h"

static poly::Log g_log;

struct M {
    int n;
    M(int n) : n(n) { g_log.add_num("M+", n); if (n == 99) throw n; }
    ~M() { g_log.add_num("~M", n); }
};

struct Base {
    Base() { g_log.add("Base+"); }
    ~Base() { g_log.add("~Base"); }
};

struct Derived : Base {
    M a, b, c;
    Derived(int fail) : a(1), b(fail), c(3) { g_log.add("Derived body"); }
    ~Derived() { g_log.add("~Derived"); }
};

struct Guarded {
    M m;
    Guarded(int n) try : m(n) { g_log.add("Guarded body"); } catch (int e) { g_log.add_num("ftb", e); }
};

static void test(poly::Test& t) {
    try {
        Derived d(99);
    } catch (int) {
        g_log.add("catch");
    }
    t.expect_str("constructed bases and members destroyed in reverse order", g_log.str(), "Base+ M+1 M+99 ~M1 ~Base catch");
    g_log.clear();
    try {
        Derived d(2);
        g_log.add("built");
    } catch (int) {
        g_log.add("no");
    }
    t.expect_str("success path", g_log.str(), "Base+ M+1 M+2 M+3 Derived body built ~Derived ~M3 ~M2 ~M1 ~Base");
    g_log.clear();
    try {
        Guarded g(99);
    } catch (int e) {
        g_log.add_num("rethrown", e);
    }
    t.expect_str("a function-try-block of a constructor rethrows", g_log.str(), "M+99 ftb99 rethrown99");
}
POLY_TEST_MAIN(test)
