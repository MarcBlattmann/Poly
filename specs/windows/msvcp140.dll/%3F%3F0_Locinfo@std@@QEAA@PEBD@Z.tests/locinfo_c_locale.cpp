// poly-cxx: compiler=msvc cl=/Od,/MD
// msvcp140.dll std::_Locinfo (??0_Locinfo@std@@QEAA@PEBD@Z, ??0_Locinfo@std@@QEAA@HPEBD@Z, ??1_Locinfo@std@@QEAA@XZ, ?_Locinfo_ctor@, ?_Locinfo_dtor@, ?_Locinfo_Addcats@, ?_Addcats@, ?_Getname@, ?_Getlconv@, ?_Getfalse@, ?_Gettrue@, ?_Getdays@, ?_Getmonths@, ?_Getctype@, ?_Getcvt@, ?_Getcoll@, ?_Getdateorder@, ?_Gettnames@): what the C locale's information object holds.
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
static void g1(poly::Test& t) {
    t.value("sizeof(_Locinfo)", sizeof(std::_Locinfo));
    std::_Locinfo li;
    t.expect_str("the default locale is C", li._Getname(), "C");
    t.expect_str("_Getfalse()", li._Getfalse(), "false");
    t.expect_str("_Gettrue()", li._Gettrue(), "true");
    t.expect_str("_Getdays()", li._Getdays(), ":Sun:Sunday:Mon:Monday:Tue:Tuesday:Wed:Wednesday:Thu:Thursday:Fri:Friday:Sat:Saturday");
    t.expect_str("_Getmonths()", li._Getmonths(), ":Jan:January:Feb:February:Mar:March:Apr:April:May:May:Jun:June:Jul:July:Aug:August:Sep:September:Oct:October:Nov:November:Dec:December");
    t.value("_Getdateorder()", li._Getdateorder());
    const lconv* lc = li._Getlconv();
    t.check("_Getlconv() is a table", lc != 0, 0);
    t.expect_str("decimal_point", lc->decimal_point, ".");
    t.expect_str("thousands_sep", lc->thousands_sep, "");
    t.expect_str("grouping", lc->grouping, "");
    t.expect_str("int_curr_symbol", lc->int_curr_symbol, "");
    t.expect_str("currency_symbol", lc->currency_symbol, "");
    t.expect_str("mon_decimal_point", lc->mon_decimal_point, "");
    t.expect_str("mon_thousands_sep", lc->mon_thousands_sep, "");
    t.expect_str("mon_grouping", lc->mon_grouping, "");
    t.expect_str("positive_sign", lc->positive_sign, "");
    t.expect_str("negative_sign", lc->negative_sign, "");
    t.expect("int_frac_digits is CHAR_MAX", lc->int_frac_digits, 127);
    t.expect("frac_digits is CHAR_MAX", lc->frac_digits, 127);
    t.expect("p_cs_precedes", lc->p_cs_precedes, 127);
    t.expect("p_sep_by_space", lc->p_sep_by_space, 127);
    t.expect("n_cs_precedes", lc->n_cs_precedes, 127);
    t.expect("n_sep_by_space", lc->n_sep_by_space, 127);
    t.expect("p_sign_posn", lc->p_sign_posn, 127);
    t.expect("n_sign_posn", lc->n_sign_posn, 127);
    _Ctypevec cv = li._Getctype();
    t.value("_Ctypevec._Page", cv._Page);
    t.check("_Ctypevec._Table is a 256-entry table", cv._Table != 0, 0);
    t.value("_Ctypevec._Delfl", cv._Delfl);
    t.value("_Ctypevec._LocaleName is null", cv._LocaleName == 0);
    t.expect_hex("table[' ']", (unsigned short)cv._Table[' '], 0x48);
    t.expect_hex("table['a']", (unsigned short)cv._Table['a'], 0x82);
    t.expect_hex("table['A']", (unsigned short)cv._Table['A'], 0x81);
    t.expect_hex("table['0']", (unsigned short)cv._Table['0'], 0x84);
    _Cvtvec cvt = li._Getcvt();
    t.value("_Cvtvec size", sizeof(_Cvtvec));
    t.value("_Cvtvec._Page", cvt._Page);
    t.value("_Cvtvec._Mbcurmax", cvt._Mbcurmax);
    t.value("_Cvtvec._Isclocale", cvt._Isclocale);
    _Collvec cl = li._Getcoll();
    t.value("_Collvec size", sizeof(_Collvec));
    t.value("_Collvec._Page", cl._Page);
    t.value("_Collvec._LocaleName is null", cl._LocaleName == 0);
}
static void g2(poly::Test& t) {
    std::_Locinfo c("C");
    t.expect_str("explicit C", c._Getname(), "C");
    std::_Locinfo c2(6, "C");
    t.expect_str("(category, name)", c2._Getname(), "C");
    std::_Locinfo c3(std::locale::all, "C");
    t.expect_str("(all, C)", c3._Getname(), "C");
    c3._Addcats(std::locale::ctype, "C");
    t.expect_str("_Addcats with the same name keeps the name", c3._Getname(), "C");
    bool thrown = false;
    std::string what;
    try { std::_Locinfo n((const char*)0); } catch (const std::runtime_error& e) { thrown = true; what = e.what(); }
    t.check("a null name throws std::runtime_error", thrown, 0);
    t.expect_str("its text", what.c_str(), "bad locale name");
    thrown = false;
    try { std::_Locinfo n(6, (const char*)0); } catch (const std::runtime_error& e) { thrown = true; }
    t.check("a null name with a category throws as well", thrown, 0);
    thrown = false;
    try { c._Addcats(6, (const char*)0); } catch (const std::runtime_error& e) { thrown = true; }
    t.check("_Addcats with a null name throws", thrown, 0);
    // a name the C runtime does not know is not an exception of _Locinfo: the name becomes "*"
    std::_Locinfo bad("no-such-locale-name");
    t.expect_str("an unknown name gives the name \"*\"", bad._Getname(), "*");
    std::_Locinfo bad2(6, "no-such-locale-name");
    t.expect_str("also with a category", bad2._Getname(), "*");
    // the destructor of a Locinfo restores the C runtime's locale: C is still the current one
    t.expect_str("the C locale is still the program's", std::_Locinfo()._Getname(), "C");
}
static void test(poly::Test& t) {
    g1(t);
    g2(t);
}
POLY_TEST_MAIN(test)
