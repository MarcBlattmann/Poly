// poly-cxx: compiler=msvc
// type_info::name(): pointers, arrays, function pointers and members: Visual C++ spells x64 pointers "* __ptr64".
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
    NAME(int*) NAME(const int*) NAME(int* const) NAME(int**) NAME(void*) NAME(char*) NAME(const char*)
    NAME(int[4]) NAME(int(*)[4]) NAME(int(*)(int, char)) NAME(void(*)())
}
POLY_TEST_MAIN(test)
