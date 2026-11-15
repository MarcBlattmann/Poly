// poly-cxx: compiler=msvc
// __CxxFrameHandler4: `throw;` rethrows the same exception object (not a copy), also from a
// function the catch block calls; `throw e;` throws a copy of the (static type of the) caught
// object: a Derived caught as Base and rethrown by name is sliced to Base; a rethrown exception
// is caught by an outer clause for a base class; the object is destroyed once, at the end of the
// outermost catch block that handles it.
#include "poly_wtest.h"

static poly::Log g_log;

struct Base {
    int tag;
    Base(int t) : tag(t) { g_log.add_num("Base+", t); }
    Base(const Base& o) : tag(o.tag + 1000) { g_log.add_num("Basecopy", tag); }
    virtual ~Base() { g_log.add_num("~Base", tag); }
    virtual int kind() const { return 1; }
};
struct Derived : Base {
    Derived(int t) : Base(t) {}
    Derived(const Derived& o) : Base(o) { tag += 10000; g_log.add_num("Derivedcopy", tag); }
    ~Derived() { g_log.add_num("~Derived", tag); }
    int kind() const override { return 2; }
};

__declspec(noinline) static void rethrow_here() { throw; }

static void test(poly::Test& t) {
    int kind = 0;
    try {
        try {
            throw Derived(1);
        } catch (Base& b) {
            g_log.add("inner");
            throw;
        }
    } catch (Base& b) {
        kind = b.kind();
        g_log.add("outer");
    }
    t.expect("the object is still the Derived", kind, 2);
    t.expect_str("no copy, one destruction", g_log.str(), "Base+1 inner outer ~Derived1 ~Base1");
    g_log.clear();
    try {
        try {
            throw Derived(2);
        } catch (Base& b) {
            throw b;
        }
    } catch (Base& b) {
        kind = b.kind();
    }
    t.expect("`throw b` throws a Base (sliced)", kind, 1);
    t.text("throw b log", g_log.str());
    g_log.clear();
    try {
        try {
            throw Derived(3);
        } catch (...) {
            rethrow_here();
        }
    } catch (Derived& d) {
        kind = d.kind();
    }
    t.expect("rethrow from a called function", kind, 2);
    t.expect_str("same object", g_log.str(), "Base+3 ~Derived3 ~Base3");
}
POLY_TEST_MAIN(test)
