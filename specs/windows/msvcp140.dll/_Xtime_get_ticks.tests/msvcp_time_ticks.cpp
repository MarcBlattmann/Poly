// poly-cxx: compiler=msvc
// msvcp140.dll _Xtime_get_ticks (extern "C", long long): the system time in 100 ns ticks since 1970-01-01 (the
// epoch of std::chrono::system_clock), as GetSystemTimePreciseAsFileTime's FILETIME minus 116444736000000000.
#include "poly_wtest.h"

extern "C" {
__declspec(dllimport) long long __cdecl _Xtime_get_ticks();
__declspec(dllimport) void __stdcall GetSystemTimePreciseAsFileTime(unsigned long long*);
__declspec(dllimport) void __stdcall Sleep(poly_u32);
}

static long long now_ticks() {
    unsigned long long ft = 0;
    GetSystemTimePreciseAsFileTime(&ft);
    return (long long)ft - 116444736000000000ll;
}

static void test(poly::Test& t) {
    long long before = now_ticks();
    long long x = _Xtime_get_ticks();
    long long after = now_ticks();
    t.check("it lies between two readings of the system clock (100 ns ticks since 1970)", before <= x + 10000000 && x <= after + 10000000, x - before);
    t.check("it is a time of this decade (2020 to 2100)", x > 15778368000000000ll && x < 41358528000000000ll, x);
    long long a = _Xtime_get_ticks();
    Sleep(30);
    long long b = _Xtime_get_ticks();
    long long ms = (b - a) / 10000;
    t.check("30 ms of sleep move it by 20 to 2000 ms", ms >= 20 && ms <= 2000, ms);
    t.check("the unit is 100 ns: a second is about 10 million ticks", (_Xtime_get_ticks() - a) < 600000000ll, 0);
    long long prev = _Xtime_get_ticks();
    bool monotonic = true;
    for (int i = 0; i < 1000; i++) {
        long long now = _Xtime_get_ticks();
        if (now < prev) monotonic = false;
        prev = now;
    }
    t.check("it does not go back within a short loop", monotonic, 0);
}
POLY_TEST_MAIN(test)
