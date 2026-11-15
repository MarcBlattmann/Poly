// poly-cxx: compiler=msvc
// __std_type_info_compare called directly (vcruntime140.dll) on hand-made type_info data: 0 for equal decorated names,
// negative or positive otherwise; the result's sign orders the names as strcmp does. The values are recorded.
#include "poly_wtest.h"

struct Data { const char* undecorated; char decorated[40]; };
extern "C" __declspec(dllimport) int __cdecl __std_type_info_compare(const Data*, const Data*);

static Data make(const char* decorated) {
    Data d = {0, {0}};
    int n = 0;
    while (decorated[n]) { d.decorated[n] = decorated[n]; n++; }
    return d;
}
static int sign(int v) { return v < 0 ? -1 : v > 0; }
static int cmp(const char* a, const char* b) {
    while (*a && *a == *b) { a++; b++; }
    return (unsigned char)*a - (unsigned char)*b;
}

static void pair(poly::Test& t, const char* a, const char* b) {
    Data x = make(a), y = make(b);
    int got = __std_type_info_compare(&x, &y);
    t.expect(a, sign(got), sign(cmp(a, b)));
    poly::Line().str("value compare ").str(a).str(" ").str(b).str(" ").dec(got).end();
}

static void test(poly::Test& t) {
    pair(t, ".H", ".H");
    pair(t, ".H", ".I");
    pair(t, ".I", ".H");
    pair(t, ".?AUS@@", ".?AUK@@");
    pair(t, ".?AUK@@", ".?AUS@@");
    pair(t, ".PEAH", ".PEAD");
    pair(t, ".H", ".H1");
    pair(t, ".H1", ".H");
    pair(t, ".", ".H");
    pair(t, ".?AUS@@", ".?AUS@@");
    Data a = make(".H");
    t.expect("a type_info compared with itself", __std_type_info_compare(&a, &a), 0);
    Data u = make(".H");
    u.undecorated = "int";
    Data v = make(".H");
    t.expect("the undecorated pointer is not compared", __std_type_info_compare(&u, &v), 0);
}
POLY_TEST_MAIN(test)
