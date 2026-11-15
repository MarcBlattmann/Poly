// poly-cxx: compiler=msvc
// __RTDynamicCast: downcast and sideways casts of pointers in a class with two bases (D : B, C; A twice),
// null stays null, a downcast to a type the object is not gives null.
#include "poly_wtest.h"
#include <typeinfo>

struct A { virtual ~A() {} int a = 1; };
struct B : A { int b = 2; };
struct C : A { int c = 3; };
struct D : B, C { int d = 4; };         // two A subobjects (ambiguous)
struct Solo { virtual ~Solo() {} };
struct E : Solo, D {};                   // D not at offset 0
struct VA { virtual ~VA() {} int va = 5; };
struct V1 : virtual VA { int v1 = 6; };
struct V2 : virtual VA { int v2 = 7; };
struct VD : V1, V2 { int vd = 8; };
struct PrivB : private A { A* self() { return this; } };
struct ProtB : protected A { A* self() { return this; } };
struct PubD : PrivB { ::A* self() { return PrivB::self(); } };
struct Plain { int x; };                 // not polymorphic

template <class T>
static long off(const void* base, T* p) {
    return p ? (const char*)p - (const char*)base : -1;
}

static poly::Log g_log;
static const char* g_phase = "";
struct Probe : A {
    Probe() { check("ctor"); }
    ~Probe() { check("dtor"); }
    void check(const char* when) {
        // inside a constructor or destructor the object's dynamic type is Probe
        g_log.add(when);
        g_log.add(dynamic_cast<Probe*>(static_cast<A*>(this)) ? "is-Probe" : "not-Probe");
    }
};
struct ProbeDerived : Probe {
    ProbeDerived() { g_log.add(dynamic_cast<ProbeDerived*>(static_cast<A*>(this)) ? "pd-ok" : "pd-null"); }
};

static void test(poly::Test& t) {
    D d;
    B* pb = &d;
    C* pc = &d;
    A* pab = pb;     // the A inside B
    A* pac = pc;     // the A inside C
    t.expect("downcast B* -> D*", off(&d, dynamic_cast<D*>(pb)), 0);
    t.expect("downcast C* -> D*", off(&d, dynamic_cast<D*>(pc)), 0);
    t.expect("downcast A* (the one in B) -> D*", off(&d, dynamic_cast<D*>(pab)), 0);
    t.expect("downcast A* (the one in C) -> D*", off(&d, dynamic_cast<D*>(pac)), 0);
    t.expect("sideways B* -> C*", off(&d, dynamic_cast<C*>(pb)), (long)((char*)pc - (char*)&d));
    t.expect("sideways C* -> B*", off(&d, dynamic_cast<B*>(pc)), 0);
    t.expect("from the A in B to C: found from the complete object, though A is ambiguous in D", off(&d, dynamic_cast<C*>(pab)), (long)((char*)pc - (char*)&d));
    t.expect("A* -> B* succeeds from the A inside B", off(&d, dynamic_cast<B*>(pab)), 0);
    t.expect("from the A in C to B: found from the complete object", off(&d, dynamic_cast<B*>(pac)), 0);
    t.expect("null pointer stays null", off(&d, dynamic_cast<D*>((B*)nullptr)), -1);
    B justb;
    t.expect("downcast to a type the object is not", off(&justb, dynamic_cast<D*>((A*)&justb)), -1);
    t.expect("to its own type", off(&justb, dynamic_cast<B*>((A*)&justb)), 0);
    t.expect("to the same pointer type", off(&justb, dynamic_cast<A*>((A*)&justb)), 0);
}
POLY_TEST_MAIN(test)
