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
// msvcp140_1.dll _Aligned_new_delete_resource: the object's virtual table is (destructor, do_allocate, do_deallocate, do_is_equal); calls through the raw slots.
static void test(poly::Test& t) {
    MR* r = std::pmr::new_delete_resource();
    t.check("the exported function returns the same object as new_delete_resource()", (MR*)std::pmr::_Aligned_new_delete_resource() == r, 0);
    t.check("it returns the same object every time", std::pmr::new_delete_resource() == r, 0);
    void** v = *(void***)r;
    AllocFn alloc = (AllocFn)v[1];
    DeallocFn dealloc = (DeallocFn)v[2];
    EqFn eq = (EqFn)v[3];
    char* p = (char*)alloc(r, 200, 8);
    t.check("do_allocate(200, 8) is not null", p != 0, 0);
    for (int i = 0; i < 200; i++) p[i] = (char)i;
    int ok = 1;
    for (int i = 0; i < 200; i++) if (p[i] != (char)i) ok = 0;
    t.check("the memory is writable and keeps its contents", ok, 0);
    t.check("a block for alignment 8 is 16-byte aligned", ((unsigned long long)p & 15) == 0, 0);
    dealloc(r, p, 200, 8);
    t.check("slot 3 is do_is_equal: itself", eq(r, r), 0);
    t.check("do_is_equal: the unaligned new_delete is not equal", !eq(r, std::pmr::_Unaligned_new_delete_resource()), 0);
    t.check("do_is_equal: the null resource is not equal", !eq(r, std::pmr::null_memory_resource()), 0);
    t.check("is_equal() through the inline wrapper agrees", r->is_equal(*r) && !r->is_equal(*std::pmr::null_memory_resource()), 0);
    t.check("operator== of two references to the same one", *r == *std::pmr::new_delete_resource(), 0);
    t.check("the unaligned resource's table is a different table", *(void***)std::pmr::_Unaligned_new_delete_resource() != v, 0);
    char* a = (char*)alloc(r, 32, 16);
    char* b = (char*)alloc(r, 32, 16);
    t.check("two live blocks differ", a != b && a != 0 && b != 0, 0);
    dealloc(r, a, 32, 16);
    dealloc(r, b, 32, 16);
    char* z = (char*)r->allocate(0, 1);
    t.check("allocate(0, 1) returns a pointer", z != 0, 0);
    r->deallocate(z, 0, 1);
}
POLY_TEST_MAIN(test)
