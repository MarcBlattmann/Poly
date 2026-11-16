// poly-cxx: compiler=msvc
#include "poly_wtest.h"
#include <memory_resource>
#include <new>
#include <stdexcept>

void* __cdecl operator new(unsigned long long n) { return malloc(n); }
using MR = std::pmr::memory_resource;
// The documented virtual interface by slot number (0 destructor, 1 do_allocate, 2 do_deallocate, 3 do_is_equal).
typedef void* (__cdecl *AllocFn)(MR*, unsigned long long, unsigned long long);
typedef void (__cdecl *DeallocFn)(MR*, void*, unsigned long long, unsigned long long);
typedef bool (__cdecl *EqFn)(const MR*, const MR*);
static void** vtab(MR* r) { return *(void***)r; }

// msvcp140_1.dll _Aligned_get_default_resource / _Unaligned_get_default_resource: the process-wide default resource before anyone sets one.
static void test(poly::Test& t) {
    MR* a = std::pmr::get_default_resource();
    t.check("aligned default is not null", a != 0, 0);
    t.check("it is the aligned new/delete resource", a == std::pmr::new_delete_resource(), 0);
    t.check("it is not the null resource", a != std::pmr::null_memory_resource(), 0);
    t.check("a second call returns the same object", std::pmr::get_default_resource() == a, 0);
    MR* u = std::pmr::_Unaligned_get_default_resource();
    t.check("unaligned default is not null", u != 0, 0);
    t.check("unaligned default is the unaligned new/delete resource", u == std::pmr::_Unaligned_new_delete_resource(), 0);
    t.check("the two new/delete resources are different objects", u != a, 0);
    t.check("the two get calls do not share the object", std::pmr::_Unaligned_get_default_resource() == u, 0);
    t.check("an object's first word is a vtable pointer", *(void**)a != 0 && *(void**)u != 0, 0);
    t.check("both kinds have separate vtables", *(void**)a != *(void**)u, 0);
    t.check("the objects are 8-byte aligned", (((unsigned long long)a | (unsigned long long)u) & 7) == 0, 0);
}
POLY_TEST_MAIN(test)
