// poly-cxx: compiler=msvc
// __RTDynamicCast: the polymorphic base is not at offset 0 (E : Solo, D): casts add and remove the offsets.
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
    E e;
    D* pd = &e;
    t.expect("E: D is not at offset 0", (long)((char*)pd - (char*)&e) > 0, 1);
    t.expect("E: A (in B) -> E*", off(&e, dynamic_cast<E*>((A*)(B*)pd)), 0);
    t.expect("E: Solo* -> D* sideways", off(&e, dynamic_cast<D*>((Solo*)&e)), (long)((char*)pd - (char*)&e));
    t.expect("E: C* -> Solo* sideways", off(&e, dynamic_cast<Solo*>((C*)pd)), 0);
    t.expect("E: Solo* -> C* sideways", off(&e, dynamic_cast<C*>((Solo*)&e)), (long)((char*)(C*)pd - (char*)&e));
    t.value("offset of D in E", (long)((char*)pd - (char*)&e));
}
POLY_TEST_MAIN(test)
