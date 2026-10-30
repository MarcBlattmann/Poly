// _CxxThrowException with a ThrowInfo the program builds itself at run
// time (in its own writable data, offsets from __ImageBase), not the
// compiler's. The runtime goes only by the ThrowInfo: a catch clause
// matches a catchable type whose type descriptor has the same decorated
// name as the clause's (a different descriptor at another address with a
// null vftable pointer will do); the array is searched in order; a by-value
// catch calls the catchable type's copy function (this = catch object,
// argument = exception object); the ThrowInfo's destructor is called with
// the exception object when the catch block ends; attributes bit 0 (const)
// keeps a non-const pointer clause from matching.
#include "poly_wtest.h"

extern "C" char __ImageBase;
extern "C" __declspec(noreturn) void _CxxThrowException(void* object, void* throw_info);

static poly::Log g_log;

static poly_u32 rva(const void* p) { return (poly_u32)((const char*)p - &__ImageBase); }

struct TypeDescriptor {
    const void* vftable;
    void* spare;
    char name[24];
};
struct CatchableType {
    poly_u32 properties;
    poly_u32 type;
    int mdisp, pdisp, vdisp;
    int size;
    poly_u32 copy;
};
struct CatchableTypeArray {
    int count;
    poly_u32 types[2];
};
struct ThrowInfo {
    poly_u32 attributes;
    poly_u32 destructor;
    poly_u32 forward;
    poly_u32 types;
};

static void set_name(TypeDescriptor& td, const char* n) {
    int i = 0;
    for (; n[i] && i < 23; i++) td.name[i] = n[i];
    td.name[i] = 0;
}

struct Hand {
    int v;
    Hand(int x) : v(x) {}
    Hand(const Hand& o) : v(o.v) { g_log.add("compiler-copy"); }
    ~Hand() { g_log.add_num("~Hand", v); }
};

static const void* g_copy_dest;
static const void* g_copy_src;
static const void* g_dtor_this;
static void hand_copy(void* dest, const void* src) {
    g_copy_dest = dest;
    g_copy_src = src;
    ((Hand*)dest)->v = ((const Hand*)src)->v + 1000;
    g_log.add("our-copy");
}
static void hand_dtor(void* self) {
    g_dtor_this = self;
    g_log.add_num("our-dtor", ((Hand*)self)->v);
}

static TypeDescriptor td_int, td_hand, td_other, td_intptr;
static CatchableType ct_int, ct_hand, ct_other, ct_intptr;
static CatchableTypeArray cta_int, cta_hand, cta_two, cta_intptr;
static ThrowInfo ti_int, ti_hand, ti_two, ti_const_intptr;

static void build() {
    set_name(td_int, ".H");
    td_int.vftable = 0;
    set_name(td_hand, ".?AUHand@@");
    td_hand.vftable = poly_type_info_vftable;
    set_name(td_other, ".?AUOther@@");
    td_other.vftable = poly_type_info_vftable;
    set_name(td_intptr, ".PEAH");
    td_intptr.vftable = poly_type_info_vftable;

    ct_int = {1, rva(&td_int), 0, -1, 0, 4, 0};
    ct_hand = {0, rva(&td_hand), 0, -1, 0, (int)sizeof(Hand), rva((const void*)hand_copy)};
    ct_other = {1, rva(&td_other), 0, -1, 0, 4, 0};
    ct_intptr = {1, rva(&td_intptr), 0, -1, 0, 8, 0};

    cta_int.count = 1;
    cta_int.types[0] = rva(&ct_int);
    cta_hand.count = 1;
    cta_hand.types[0] = rva(&ct_hand);
    cta_two.count = 2;
    cta_two.types[0] = rva(&ct_other);
    cta_two.types[1] = rva(&ct_int);
    cta_intptr.count = 1;
    cta_intptr.types[0] = rva(&ct_intptr);

    ti_int = {0, 0, 0, rva(&cta_int)};
    ti_hand = {0, rva((const void*)hand_dtor), 0, rva(&cta_hand)};
    ti_two = {0, 0, 0, rva(&cta_two)};
    ti_const_intptr = {1, 0, 0, rva(&cta_intptr)};
}

__declspec(noinline) static void throw_raw(void* object, ThrowInfo* ti) { _CxxThrowException(object, ti); }

static void test(poly::Test& t) {
    build();

    int value = 41;
    int r = 0;
    try {
        throw_raw(&value, &ti_int);
    } catch (long) {
        r = 1;
    } catch (int v) {
        r = v + 1;
    }
    t.expect("matched by name: our .H is int", r, 42);

    Hand h(7);
    g_log.clear();
    r = 0;
    try {
        throw_raw(&h, &ti_hand);
    } catch (Hand c) {
        r = c.v;
        t.check("copy function's this is the catch object", g_copy_dest == (const void*)&c, 0);
        t.check("its argument is the exception object", g_copy_src == (const void*)&h, 0);
    }
    t.expect("our copy function made the catch object", r, 1007);
    t.expect_str("our copy, the catch object's destructor, then our destructor", g_log.str(),
                 "our-copy ~Hand1007 our-dtor7");
    t.check("our destructor got the exception object", g_dtor_this == (const void*)&h, 0);

    g_log.clear();
    r = 0;
    try {
        throw_raw(&h, &ti_hand);
    } catch (Hand& ref) {
        r = &ref == &h ? 1 : -1;
    }
    t.expect("by reference: the object itself", r, 1);
    t.expect_str("no copy; our destructor at the end", g_log.str(), "our-dtor7");

    r = 0;
    value = 9;
    try {
        throw_raw(&value, &ti_two);
    } catch (Hand&) {
        r = 1;
    } catch (int v) {
        r = v;
    }
    t.expect("second entry of the array matches", r, 9);

    int target = 77;
    int* p = &target;
    r = 0;
    try {
        throw_raw(&p, &ti_const_intptr);
    } catch (int*) {
        r = 1;
    } catch (const int* cp) {
        r = *cp;
    }
    t.expect("const attribute: not int*, but const int*", r, 77);
}
POLY_TEST_MAIN(test)
