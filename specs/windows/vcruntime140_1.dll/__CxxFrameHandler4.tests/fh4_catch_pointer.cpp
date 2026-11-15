// poly-cxx: compiler=msvc
// __CxxFrameHandler4: pointers as exception objects: the pointer is copied; a Derived* is taken
// by a Base* clause (adjusted for the base subobject's offset), by a void* clause, and by a const
// clause; nullptr only by void*; a pointer to a class is not taken by a clause for the class.
#include "poly_wtest.h"

struct A { int a; virtual ~A() {} };
struct B { int b; virtual ~B() {} };
struct C : A, B { int c; };

static C* g_cp;
static int g_int = 7;

__declspec(noinline) static void throw_c() { throw g_cp; }
__declspec(noinline) static void throw_null() { throw nullptr; }
__declspec(noinline) static void throw_int_ptr() { throw &g_int; }
__declspec(noinline) static void throw_cint_ptr() { throw (const int*)&g_int; }

static void test(poly::Test& t) {
    C c;
    g_cp = &c;
    int which = 0;
    try { throw_c(); } catch (B* p) { which = 2; t.check("B* adjusted to the B subobject", (char*)p == (char*)g_cp + sizeof(A), 0); }
    t.expect("C* caught as the second base B*", which, 2);
    which = 0;
    try { throw_c(); } catch (A* p) { which = 1; t.check("A* is the object", (char*)p == (char*)g_cp, 0); }
    t.expect("C* caught as the first base A*", which, 1);
    which = 0;
    try { throw_c(); } catch (C* p) { which = 3; t.check("C* is the object", p == g_cp, 0); }
    t.expect("C* caught as C*", which, 3);
    which = 0;
    try { throw_c(); } catch (void* p) { which = 4; t.check("void* is the object", p == (void*)g_cp, 0); }
    t.expect("C* caught as void*", which, 4);
    which = 0;
    try { throw_c(); } catch (C) { which = 5; } catch (A) { which = 6; } catch (int*) { which = 7; } catch (...) { which = 8; }
    t.expect("a pointer to a class is not the class", which, 8);
    which = 0;
    try { throw_null(); } catch (int* p) { which = 1; t.check("null as int*", p == 0, 0); } catch (void* p) { which = 2; t.check("null as void*", p == 0, 0); } catch (...) { which = 3; }
    t.value("nullptr taken by", which);
    which = 0;
    try { throw_int_ptr(); } catch (const int* p) { which = 1; t.check("int* as const int*", p == &g_int, 0); } catch (...) { which = 2; }
    t.expect("int* taken by const int*", which, 1);
    which = 0;
    try { throw_cint_ptr(); } catch (int* p) { which = 1; } catch (...) { which = 2; }
    t.expect("const int* not taken by int*", which, 2);
}
POLY_TEST_MAIN(test)
