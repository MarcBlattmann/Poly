// poly-cxx: compiler=msvc
// type_info::name(): classes, structs, unions, enums, namespaces and templates.
#include "poly_wtest.h"
#include <typeinfo>

// type_info::name() passes this list head to __std_type_info_name (the C runtime normally defines it).
struct __type_info_node {
    void* _MemPtr;
    __type_info_node* _Next;
};
alignas(16) __type_info_node __type_info_root_node = {nullptr, nullptr};

struct S { };
class K { };
union U { int i; };
enum En { };
enum class EC { };
namespace ns { struct N { }; namespace inner { class M { }; } }
template <class T> struct Tpl { };
struct Base { virtual ~Base() {} };

static int cmp(const char* a, const char* b) {
    while (*a && *a == *b) { a++; b++; }
    return (unsigned char)*a - (unsigned char)*b;
}

#define NAME(T) t.text("name " #T, typeid(T).name());
#define RAW(T) t.text("raw  " #T, typeid(T).raw_name());

static void test(poly::Test& t) {
    NAME(S) NAME(K) NAME(U) NAME(En) NAME(EC) NAME(S*) NAME(const S&) NAME(S&&)
    NAME(ns::N) NAME(ns::inner::M) NAME(Tpl<int>) NAME(Tpl<S*>) NAME(Tpl<ns::N>)
    NAME(Base) NAME(int S::*) NAME(void (S::*)(int))
}
POLY_TEST_MAIN(test)
