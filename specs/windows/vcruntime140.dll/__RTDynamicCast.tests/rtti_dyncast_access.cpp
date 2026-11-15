// poly-cxx: compiler=msvc
// __RTDynamicCast: private and protected bases are not paths for a downcast: the result is null.
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
    PrivB priv;
    ProtB prot;
    t.expect("downcast from a private base is not allowed: null", off(&priv, dynamic_cast<PrivB*>(priv.self())), -1);
    t.expect("downcast from a protected base is not allowed: null", off(&prot, dynamic_cast<ProtB*>(prot.self())), -1);
    PubD pubd;
    t.expect("a public derived class of a private-base class: the private A is not a path", off(&pubd, dynamic_cast<PubD*>(pubd.self())), -1);
}
POLY_TEST_MAIN(test)
