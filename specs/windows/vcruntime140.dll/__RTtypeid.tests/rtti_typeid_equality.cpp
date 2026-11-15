// poly-cxx: compiler=msvc
// type_info equality (__std_type_info_compare): typeid ignores top-level cv and references, keeps pointers and
// pointee constness.
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
    A* pa = &b;
    (void)pa;
    t.check("cv and references are ignored", typeid(const volatile B&) == typeid(B), 0);
    t.check("pointers are not ignored", typeid(B*) != typeid(B), 0);
    t.check("pointer constness matters one level down", typeid(const B*) != typeid(B*), 0);

}
POLY_TEST_MAIN(test)
