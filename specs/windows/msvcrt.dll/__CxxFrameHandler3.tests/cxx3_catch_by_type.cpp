// __CxxFrameHandler3: a catch clause takes an exception of exactly its type
// (top-level const and references ignored); the first matching clause of
// a try block wins, the ones that do not match are skipped; no arithmetic
// conversions (int is not long, not unsigned, not char).
#include "poly_wtest.h"

enum Color { Red = 1, Green = 2 };

__declspec(noinline) static void throw_int(int v) { throw v; }
__declspec(noinline) static void throw_unsigned(unsigned v) { throw v; }
__declspec(noinline) static void throw_char(char v) { throw v; }
__declspec(noinline) static void throw_long_long(long long v) { throw v; }
__declspec(noinline) static void throw_enum(Color c) { throw c; }

static int which_int(int v) {
    try {
        throw_int(v);
    } catch (long) {
        return 1;
    } catch (unsigned) {
        return 2;
    } catch (char) {
        return 3;
    } catch (int x) {
        return x == v ? 4 : -4;
    } catch (...) {
        return 5;
    }
    return 0;
}

static void test(poly::Test& t) {
    t.expect("int skips long, unsigned, char", which_int(42), 4);

    int r = 0;
    try {
        throw_unsigned(7u);
    } catch (int) {
        r = 1;
    } catch (unsigned x) {
        r = x == 7u ? 2 : -2;
    }
    t.expect("unsigned is not int", r, 2);

    r = 0;
    try {
        throw_char('q');
    } catch (int) {
        r = 1;
    } catch (char c) {
        r = c == 'q' ? 2 : -2;
    }
    t.expect("char is not int", r, 2);

    r = 0;
    try {
        throw_long_long(1LL << 40);
    } catch (long) {
        r = 1;
    } catch (long long x) {
        r = x == (1LL << 40) ? 2 : -2;
    }
    t.expect("long long is not long", r, 2);

    r = 0;
    try {
        throw_enum(Green);
    } catch (int) {
        r = 1;
    } catch (Color c) {
        r = c == Green ? 2 : -2;
    }
    t.expect("enum is not int", r, 2);

    r = 0;
    try {
        throw_int(5);
    } catch (const int& x) {
        r = x;
    }
    t.expect("const int& takes int", r, 5);

    r = 0;
    try {
        throw_int(6);
    } catch (int& x) {
        x += 1;
        r = x;
    }
    t.expect("int& takes int", r, 7);

    r = 0;
    try {
        throw_int(8);
    } catch (const volatile int x) {
        r = x;
    }
    t.expect("cv int by value takes int", r, 8);

    r = 0;
    try {
        throw_int(9);
    } catch (int) {
        r = 1;
    } catch (...) {
        r = 2;
    }
    t.expect("first match wins over catch(...)", r, 1);

    r = 0;
    try {
        throw_int(10);
    } catch (...) {
        r = 3;
    }
    t.expect("catch(...) takes int", r, 3);
}
POLY_TEST_MAIN(test)
