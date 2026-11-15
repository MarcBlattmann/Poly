// poly-cxx: compiler=msvc
// __RTtypeid: typeid of a polymorphic glvalue is the type of the most derived object (through bases,
// references and virtual bases); of a pointer or a non-polymorphic expression it is static.
#include "poly_wtest.h"
#include <typeinfo>

struct A { virtual ~A() {} };
struct B : A {};
struct C : B {};
struct VA { virtual ~VA() {} };
struct V1 : virtual VA {};
struct V2 : virtual VA {};
struct VD : V1, V2 {};
struct Plain { int x; };

static void test(poly::Test& t) {
    B b;
    C c;
    A* pa = &b;
    A* pc = &c;
    A& rc = c;
    t.check("typeid of *pa is B", typeid(*pa) == typeid(B), 0);
    t.check("typeid of *pc is C, not A", typeid(*pc) == typeid(C) && typeid(*pc) != typeid(A), 0);
    t.check("typeid of a reference to the base is the dynamic type", typeid(rc) == typeid(C), 0);
    t.check("typeid of the pointer itself is static", typeid(pa) == typeid(A*), 0);
    VD vd;
    VA* pv = &vd;
    t.check("through a virtual base", typeid(*pv) == typeid(VD), 0);
    V2& v2 = vd;
    t.check("through the second branch", typeid(v2) == typeid(VD), 0);
    Plain plain;
    Plain* pp = &plain;
    t.check("non-polymorphic: static type", typeid(*pp) == typeid(Plain), 0);
}
POLY_TEST_MAIN(test)
