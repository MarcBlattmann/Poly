// poly-cxx: compiler=msvc
// __RTDynamicCast: virtual bases (VD : V1, V2 : virtual VA): the offsets come from the vbtable.
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
    VD vd;
    VA* pva = &vd;
    V1* pv1 = &vd;
    V2* pv2 = &vd;
    t.expect("virtual base VA* -> VD*", off(&vd, dynamic_cast<VD*>(pva)), 0);
    t.expect("VA* -> V1*", off(&vd, dynamic_cast<V1*>(pva)), 0);
    t.expect("VA* -> V2*", off(&vd, dynamic_cast<V2*>(pva)), (long)((char*)pv2 - (char*)&vd));
    t.expect("V1* -> V2* sideways through the virtual base", off(&vd, dynamic_cast<V2*>(pv1)), (long)((char*)pv2 - (char*)&vd));
    t.expect("V2* -> V1*", off(&vd, dynamic_cast<V1*>(pv2)), 0);
    t.expect("V1* -> VA* upcast (virtual)", off(&vd, dynamic_cast<VA*>(pv1)), (long)((char*)pva - (char*)&vd));
    t.expect("V2* -> VD*", off(&vd, dynamic_cast<VD*>(pv2)), 0);
    t.value("offset of V2 in VD", (long)((char*)pv2 - (char*)&vd));
    t.value("offset of VA in VD", (long)((char*)pva - (char*)&vd));
}
POLY_TEST_MAIN(test)
