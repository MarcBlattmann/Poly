// __CxxFrameHandler3: a virtual base is found through the thrown object's
// virtual base table (the catchable type's pdisp/vdisp/mdisp); catching a
// class that has virtual bases by value calls its copy constructor with
// the "most derived" flag, so the virtual base is copied once.
#include "poly_wtest.h"

static poly::Log g_log;
static const void* g_thrown;

struct VB {
    int v;
    VB(int x = 0) : v(x) {}
    VB(const VB& o) : v(o.v) { g_log.add_num("VB-copy", v); }
};
struct A1 : virtual VB {
    int a = 1;
    A1() : VB(0) {}
};
struct A2 : virtual VB {
    int b = 2;
    A2() : VB(0) {}
};
struct Bottom : A1, A2 {
    int c = 3;
    Bottom(int x) : VB(x) { g_thrown = this; }
    Bottom(const Bottom& o) : VB(o), A1(o), A2(o), c(o.c) { g_log.add("Bottom-copy"); }
};

__declspec(noinline) static void throw_bottom(int v) { throw Bottom(v); }

static void test(poly::Test& t) {
    int r = 0;
    try {
        throw_bottom(70);
    } catch (VB& vb) {
        const Bottom* whole = (const Bottom*)g_thrown;
        t.check("VB& is the virtual base subobject", (const void*)&vb == (const void*)static_cast<const VB*>(whole), 0);
        r = vb.v;
    }
    t.expect("VB& reads the virtual base", r, 70);

    r = 0;
    try {
        throw_bottom(71);
    } catch (A2& a2) {
        const Bottom* whole = (const Bottom*)g_thrown;
        t.check("A2& is the A2 subobject", (const void*)&a2 == (const void*)static_cast<const A2*>(whole), 0);
        r = a2.b * 100 + a2.v;
    }
    t.expect("A2& reads its own member and the shared virtual base", r, 271);

    g_log.clear();
    r = 0;
    try {
        throw_bottom(72);
    } catch (Bottom b) {
        r = b.v + b.a + b.b + b.c;
    }
    t.expect("Bottom by value", r, 72 + 1 + 2 + 3);
    t.expect_str("one copy of the virtual base, then Bottom's", g_log.str(), "VB-copy72 Bottom-copy");

    g_log.clear();
    r = 0;
    try {
        throw_bottom(73);
    } catch (VB vb) {
        r = vb.v;
    }
    t.expect("VB by value", r, 73);
    t.expect_str("VB by value copies the virtual base only", g_log.str(), "VB-copy73");
}
POLY_TEST_MAIN(test)
