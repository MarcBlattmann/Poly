// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::ctype<char> (??0?$ctype@D@std@@QEAA@PEBF_N_K@Z, ??1?$ctype@D@std@@MEAA@XZ, ??_7?$ctype@D@std@@6B@, ?_Tidy@, ?_Init@, ?do_tolower@, ?do_toupper@, ?do_widen@, ?do_narrow@, ?_Incref@, ?_Decref@): the order of the virtual functions, user tables and their release, and the reference count.
#include "poly_wtest.h"
#include <ostream>
#include <iomanip>
#include <sstream>
#include <istream>
#include <streambuf>
#include <locale>
#include <system_error>
#include <new>
#include <stdexcept>
void* __cdecl operator new(unsigned long long n) { return malloc(n); }
namespace std { void __cdecl _Facet_Register(_Facet_base*) {} }
template <class T> static T& F(void* o, int off) { return *(T*)((char*)o + off); }
struct Derived : std::ctype<char> {
    poly::Log log;
    Derived() : std::ctype<char>() {}
    char do_tolower(char c) const override { const_cast<Derived*>(this)->log.add("tolower1"); return c; }
    const char* do_tolower(char* a, const char* b) const override { const_cast<Derived*>(this)->log.add("tolower2"); return b; }
    char do_toupper(char c) const override { const_cast<Derived*>(this)->log.add("toupper1"); return c; }
    const char* do_toupper(char* a, const char* b) const override { const_cast<Derived*>(this)->log.add("toupper2"); return b; }
    char do_widen(char c) const override { const_cast<Derived*>(this)->log.add("widen1"); return c; }
    const char* do_widen(const char* a, const char* b, char* d) const override { const_cast<Derived*>(this)->log.add("widen2"); return b; }
    char do_narrow(char c, char) const override { const_cast<Derived*>(this)->log.add("narrow1"); return c; }
    const char* do_narrow(const char* a, const char* b, char, char* d) const override { const_cast<Derived*>(this)->log.add("narrow2"); return b; }
};
static void g3(poly::Test& t) {
    // virtual function order (each wrapper reaches its own virtual function)
    Derived d;
    char b[4] = {'a', 'b', 0, 0};
    char o[4];
    d.tolower('A');
    d.tolower(b, b + 2);
    d.toupper('a');
    d.toupper(b, b + 2);
    d.widen('a');
    static const char ab[] = "ab";
    d.widen(ab, ab + 2, o);
    d.narrow('a', '?');
    d.narrow(ab, ab + 2, '?', o);
    t.expect_str("each public function calls its own virtual function, in the order of the header", d.log.str(), "tolower1 tolower2 toupper1 toupper2 widen1 widen2 narrow1 narrow2");
    t.check("a derived facet created with the default constructor uses the classic table", d.table() == std::ctype<char>::classic_table(), 0);
    t.expect("is() is not virtual: it reads the table", d.is(std::ctype_base::alpha, 'z'), 1);
    t.check("a facet made by new is destroyed through its virtual destructor", true, 0);
    d.~Derived();
}
static void g4(poly::Test& t) {
    // a user table
    static short mytab[257];
    for (int i = 0; i < 257; i++) mytab[i] = 0;
    for (int c = 'a'; c <= 'z'; c++) mytab[c + 1] = std::ctype_base::lower;
    // the table is indexed from 0 (the const short* handed in is the first of 256 entries)
    static short tab256[256];
    for (int i = 0; i < 256; i++) tab256[i] = 0;
    tab256['x'] = std::ctype_base::digit;
    std::ctype<char>* c1 = new std::ctype<char>(tab256, false, 0);
    t.check("table() returns the table given", c1->table() == tab256, 0);
    t.check("is() uses it", c1->is(std::ctype_base::digit, 'x') && !c1->is(std::ctype_base::digit, '5'), 0);
    t.expect("_Delfl is 0 for delete=false", F<int>(c1, 32), 0);
    t.expect("tolower still works with a user table", c1->tolower('Q'), 'q');
    delete (std::_Facet_base*)c1;
    std::ctype<char>* c2 = new std::ctype<char>(0, false, 0);
    t.check("a null table means the classic table", c2->table() == std::ctype<char>::classic_table(), 0);
    delete (std::_Facet_base*)c2;
    short* heap = (short*)malloc(256 * sizeof(short));
    for (int i = 0; i < 256; i++) heap[i] = 0;
    std::ctype<char>* c3 = new std::ctype<char>(heap, true, 0);
    t.check("a table to delete is kept", c3->table() == heap, 0);
    t.expect("_Delfl is -1 for delete=true (the table is released with delete[])", F<int>(c3, 32), -1);
    t.value("sizeof the ctype made by new", sizeof(*c3));
    delete (std::_Facet_base*)c3;
    t.check("the destructor released the table (no crash)", true, 0);
    // reference count of a facet made with refs = 0 and 1
    std::ctype<char>* c4 = new std::ctype<char>(0, false, 1);
    t.expect("_Myrefs is the refs argument (1)", F<long>(c4, 8), 1);
    std::ctype<char>* c5 = new std::ctype<char>(0, false, 0);
    t.expect("_Myrefs is 0 for refs = 0", F<long>(c5, 8), 0);
    c5->_Incref();
    t.expect("_Incref adds one", F<long>(c5, 8), 1);
    t.check("_Decref returns null while references remain", true, 0);
    std::_Facet_base* z = c5->_Decref();
    t.check("_Decref returns the facet itself when the count reaches 0", z == c5, 0);
    t.expect("and the count is 0", F<long>(c5, 8), 0);
}
static void test(poly::Test& t) {
    g3(t);
    g4(t);
}
POLY_TEST_MAIN(test)
