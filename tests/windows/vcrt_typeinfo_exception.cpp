// poly-cxx: runtime=vcruntime140
//
// vcruntime140.dll (ADR 0199): the `type_info` and `std::exception` helpers
// the inline code of Visual C++'s headers calls. Layouts from the public
// headers `vcruntime_typeinfo.h` (`__std_type_info_data`: the undecorated
// name pointer, then the decorated name) and `vcruntime_exception.h`
// (`__std_exception_data`: `_What`, `_DoFree`). The expected results follow
// from the C++ standard: equal types compare equal and hash equal
// ([type.info]), the same type described twice is the same type, and a
// copied exception keeps its text (`what()`) independently of the original.
// Implementer's tests: Windows' own results are to be recorded by a spec
// agent (ADR 0199).
#include "poly_wtest.h"

struct TypeData {
    const char* undecorated;
    char decorated[16];
};

struct ExceptionData {
    const char* what;
    bool do_free;
};

extern "C" {
__declspec(dllimport) int __std_type_info_compare(const TypeData*, const TypeData*);
__declspec(dllimport) decltype(sizeof 0) __std_type_info_hash(const TypeData*);
__declspec(dllimport) void __std_exception_copy(const ExceptionData*, ExceptionData*);
__declspec(dllimport) void __std_exception_destroy(ExceptionData*);
__declspec(dllimport) void* malloc(decltype(sizeof 0));
}

static TypeData g_a1 = {0, ".?AVA@@"};
static TypeData g_a2 = {0, ".?AVA@@"};
static TypeData g_b = {0, ".?AVB@@"};
static TypeData g_int = {0, ".H"};

static void test(poly::Test& t) {
    t.expect("a type equals itself", __std_type_info_compare(&g_a1, &g_a1), 0);
    t.expect("the same type described twice is equal", __std_type_info_compare(&g_a1, &g_a2), 0);
    int ab = __std_type_info_compare(&g_a1, &g_b);
    int ba = __std_type_info_compare(&g_b, &g_a1);
    t.check("two types differ", ab != 0, ab);
    t.check("their order is antisymmetric", (ab < 0) == (ba > 0) && ba != 0, ba);
    t.check("equal types hash equal", __std_type_info_hash(&g_a1) == __std_type_info_hash(&g_a2), 0);
    t.check("different types hash differently here", __std_type_info_hash(&g_a1) != __std_type_info_hash(&g_int), 0);

    // A literal is shared, not owned.
    ExceptionData literal = {"literal text", false};
    ExceptionData c1 = {0, true};
    __std_exception_copy(&literal, &c1);
    t.check("a literal is shared", c1.what == literal.what, 0);
    t.check("and not owned", !c1.do_free, 1);

    // Owned text is copied.
    char* text = (char*)malloc(6);
    const char* hello = "hello";
    for (int i = 0; i < 6; i++) text[i] = hello[i];
    ExceptionData owned = {text, true};
    ExceptionData c2 = {0, false};
    __std_exception_copy(&owned, &c2);
    t.check("owned text is copied", c2.what != 0 && c2.what != text, 0);
    t.check("the copy owns it", c2.do_free, 0);
    t.expect_str("with the same characters", c2.what ? c2.what : "", "hello");
    text[0] = 'j';
    t.expect_str("independent of the original", c2.what ? c2.what : "", "hello");
    __std_exception_destroy(&c2);
    t.check("destroyed: no text", c2.what == 0, 0);
    t.check("destroyed: not owned", !c2.do_free, 1);
    __std_exception_destroy(&c1);
    t.check("destroying a shared one clears it too", c1.what == 0 && !c1.do_free, 0);
}
POLY_TEST_MAIN(test)
