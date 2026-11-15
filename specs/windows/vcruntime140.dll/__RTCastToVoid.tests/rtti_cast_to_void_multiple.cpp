// poly-cxx: compiler=msvc
// __RTCastToVoid: dynamic_cast<void*> gives the most derived object whichever base subobject it starts from.
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
    E e;
    D* d = &e;
    B* b = d;
    C* c = d;
    t.expect("E from its D", off(&e, dynamic_cast<void*>(d)), 0);
    t.expect("E from the B in D", off(&e, dynamic_cast<void*>(b)), 0);
    t.expect("E from the C in D", off(&e, dynamic_cast<void*>(c)), 0);
    t.expect("E from Solo", off(&e, dynamic_cast<void*>((Solo*)&e)), 0);
    t.expect("E from the A in B", off(&e, dynamic_cast<void*>((A*)b)), 0);
    t.expect("E from the A in C", off(&e, dynamic_cast<void*>((A*)c)), 0);
    t.value("offset of D in E", off(&e, d));
    t.value("offset of C in D", off(d, c));
}
POLY_TEST_MAIN(test)
