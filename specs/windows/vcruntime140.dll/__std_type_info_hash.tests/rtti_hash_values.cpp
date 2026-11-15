// poly-cxx: compiler=msvc
// type_info::hash_code() (__std_type_info_hash): equal types give equal hashes; the values of a few types are recorded.
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
    t.value_hex("hash_code int", typeid(int).hash_code());
    t.value_hex("hash_code S", typeid(S).hash_code());
    t.value_hex("hash_code ns::N", typeid(ns::N).hash_code());
    t.value_hex("hash_code Tpl<int>", typeid(Tpl<int>).hash_code());
    t.check("equal types have equal hashes", typeid(S).hash_code() == typeid(S).hash_code(), 0);
    t.check("different types have different hashes", typeid(S).hash_code() != typeid(K).hash_code(), 0);

}
POLY_TEST_MAIN(test)
