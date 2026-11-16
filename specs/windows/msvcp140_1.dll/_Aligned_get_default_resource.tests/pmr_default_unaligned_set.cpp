// poly-cxx: compiler=msvc
#include "poly_wtest.h"
#include <memory_resource>
#include <new>
#include <stdexcept>

void* __cdecl operator new(unsigned long long n) { return malloc(n); }
using MR = std::pmr::memory_resource;
// msvcp140_1.dll _Unaligned_set_default_resource / _Unaligned_get_default_resource: the aligned and unaligned functions share ONE stored pointer; "nothing set" reads as the new/delete resource of the function's own kind.
struct Fake : MR {
    void* do_allocate(size_t, size_t) override { return 0; }
    void do_deallocate(void*, size_t, size_t) override {}
    bool do_is_equal(const MR& o) const noexcept override { return this == &o; }
};
static void test(poly::Test& t) {
    Fake f1, f2;
    MR* und = std::pmr::_Unaligned_new_delete_resource();
    MR* nd = std::pmr::new_delete_resource();
    MR* prev = std::pmr::_Unaligned_set_default_resource(&f1);
    t.check("unaligned set with nothing set returns the unaligned new_delete", prev == und, 0);
    t.check("unaligned get returns f1", std::pmr::_Unaligned_get_default_resource() == &f1, 0);
    t.check("the aligned get returns f1 too (one shared pointer)", std::pmr::get_default_resource() == &f1, 0);
    prev = std::pmr::set_default_resource(&f2);
    t.check("aligned set returns f1", prev == &f1, 0);
    t.check("unaligned get returns f2", std::pmr::_Unaligned_get_default_resource() == &f2, 0);
    prev = std::pmr::_Unaligned_set_default_resource(0);
    t.check("unaligned set(null) returns f2", prev == &f2, 0);
    t.check("unset again: unaligned get returns the unaligned new_delete", std::pmr::_Unaligned_get_default_resource() == und, 0);
    t.check("unset again: aligned get returns the aligned new_delete", std::pmr::get_default_resource() == nd, 0);
    prev = std::pmr::set_default_resource(&f2);
    t.check("aligned set when unset returns the aligned new_delete", prev == nd, 0);
    prev = std::pmr::set_default_resource(0);
    t.check("aligned set(null) returns f2", prev == &f2, 0);
    prev = std::pmr::_Unaligned_set_default_resource(0);
    t.check("unaligned set(null) when unset returns the unaligned new_delete", prev == und, 0);
    prev = std::pmr::set_default_resource(0);
    t.check("aligned set(null) when unset returns the aligned new_delete", prev == nd, 0);
}
POLY_TEST_MAIN(test)
