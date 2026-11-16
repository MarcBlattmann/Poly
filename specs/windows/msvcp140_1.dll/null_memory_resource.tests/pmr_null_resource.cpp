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
// msvcp140_1.dll null_memory_resource: allocating always throws std::bad_alloc; deallocating does nothing; equal only to itself.
static void test(poly::Test& t) {
    MR* r = std::pmr::null_memory_resource();
    t.check("not null", r != 0, 0);
    t.check("same object every time", std::pmr::null_memory_resource() == r, 0);
    t.check("not new_delete_resource()", r != std::pmr::new_delete_resource(), 0);
    void** v = *(void***)r;
    AllocFn alloc = (AllocFn)v[1];
    DeallocFn dealloc = (DeallocFn)v[2];
    EqFn eq = (EqFn)v[3];
    static const unsigned long long sizes[] = {0, 1, 100};
    static const unsigned long long aligns[] = {1, 16, 64};
    for (int i = 0; i < 3; i++) {
        bool thrown = false;
        const char* w = "";
        try { alloc(r, sizes[i], aligns[i]); } catch (const std::bad_alloc& e) { thrown = true; w = e.what(); }
        t.check("do_allocate throws std::bad_alloc", thrown, 0);
        t.expect_str("what() is 'bad allocation'", w, "bad allocation");
    }
    bool thrown = false;
    try { r->allocate(8, 8); } catch (const std::exception&) { thrown = true; }
    t.check("allocate() through the wrapper throws", thrown, 0);
    char dummy[8];
    dealloc(r, dummy, 8, 8);
    dealloc(r, 0, 0, 1);
    t.check("do_deallocate does nothing, even for a pointer it did not hand out", true, 0);
    t.check("do_is_equal: only itself", eq(r, r) && !eq(r, std::pmr::new_delete_resource()) && !eq(r, std::pmr::_Unaligned_new_delete_resource()), 0);
    t.check("operator== compares by identity", *r == *std::pmr::null_memory_resource() && !(*r == *std::pmr::new_delete_resource()), 0);
}
POLY_TEST_MAIN(test)
