// poly-cxx: compiler=msvc
// type_info::raw_name() is the decorated name from the type descriptor; name() returns the same buffer each time
// (the undecorated text is kept for the type_info's lifetime).
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
    RAW(int) RAW(int*) RAW(S) RAW(ns::inner::M) RAW(Tpl<int>) RAW(unsigned long long) RAW(const char*)

    const std::type_info& a = typeid(S);
    const char* first = a.name();
    const char* second = a.name();
    t.check("name() returns the same buffer each time", first == second, 0);
    t.check("raw_name() starts with the type descriptor's decorated name", a.raw_name()[0] == '.', 0);

}
POLY_TEST_MAIN(test)
