// poly-cxx: compiler=msvc
#include "poly_wtest.h"
#include <memory_resource>
#include <new>
#include <stdexcept>

void* __cdecl operator new(unsigned long long n) { return malloc(n); }
using MR = std::pmr::memory_resource;
typedef void* (__cdecl *AllocFn)(MR*, unsigned long long, unsigned long long);
typedef void (__cdecl *DeallocFn)(MR*, void*, unsigned long long, unsigned long long);
typedef bool (__cdecl *EqFn)(const MR*, const MR*);
// msvcp140_1.dll _Unaligned_new_delete_resource: the older resource, same slots, but an alignment above 16 is an error, std::bad_alloc.
static void test(poly::Test& t) {
    MR* r = std::pmr::_Unaligned_new_delete_resource();
    t.check("same object every time", std::pmr::_Unaligned_new_delete_resource() == r, 0);
    t.check("not the aligned one", r != std::pmr::new_delete_resource(), 0);
    void** v = *(void***)r;
    AllocFn alloc = (AllocFn)v[1];
    DeallocFn dealloc = (DeallocFn)v[2];
    EqFn eq = (EqFn)v[3];
    static const unsigned long long ok[] = {1, 2, 4, 8, 16};
    for (int i = 0; i < 5; i++) {
        void* p = alloc(r, 64, ok[i]);
        t.check("alignment up to 16 is served", p != 0 && ((unsigned long long)p & 15) == 0, 0);
        dealloc(r, p, 64, ok[i]);
    }
    static const unsigned long long bad[] = {32, 64, 4096};
    for (int i = 0; i < 3; i++) {
        bool thrown = false;
        void* p = 0;
        try { p = alloc(r, 64, bad[i]); } catch (const std::bad_alloc&) { thrown = true; }
        t.check("alignment above 16 throws std::bad_alloc", thrown && p == 0, 0);
    }
    bool thrown = false;
    try { alloc(r, 64, 32); } catch (const std::exception& e) { thrown = true; t.expect_str("what() of that bad_alloc", e.what(), "bad allocation"); }
    t.check("it is an std::exception", thrown, 0);
    t.check("do_is_equal: identity", eq(r, r) && !eq(r, std::pmr::new_delete_resource()), 0);
    void* q = r->allocate(10, 16);
    t.check("the inline wrapper allocate works for 16", q != 0, 0);
    r->deallocate(q, 10, 16);
}
POLY_TEST_MAIN(test)
