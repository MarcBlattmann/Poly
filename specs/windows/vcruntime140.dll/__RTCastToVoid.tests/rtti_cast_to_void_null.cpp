// poly-cxx: compiler=msvc
// __RTCastToVoid: null stays null; a plain object; const stays const.
#include "poly_wtest.h"

struct A { virtual ~A() {} int a; };
struct B : A { int b; };
struct C : A { int c; };
struct D : B, C { int d; };
struct Solo { virtual ~Solo() {} int s; };
struct E : Solo, D { int e; };
struct VA { virtual ~VA() {} int va; };
struct V1 : virtual VA { int v1; };
struct V2 : virtual VA { int v2; };
struct VD : V1, V2 { int vd; };

static long off(const void* base, const void* p) { return (const char*)p - (const char*)base; }

static void test(poly::Test& t) {
    A* null = nullptr;
    t.check("null stays null", dynamic_cast<void*>(null) == nullptr, 0);
    A a;
    t.expect("a plain object", off(&a, dynamic_cast<void*>(&a)), 0);
    const A* ca = &a;
    t.check("const stays const void*", dynamic_cast<const void*>(ca) == (const void*)&a, 0);
}
POLY_TEST_MAIN(test)
