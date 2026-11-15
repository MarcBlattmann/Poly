// poly-cxx: compiler=msvc
// msvcp140.dll _Query_perf_frequency and _Query_perf_counter (extern "C", long long, no arguments): the
// performance counter's frequency and value as QueryPerformanceFrequency/QueryPerformanceCounter give them
// (std::chrono::steady_clock is built on both).
#include "poly_wtest.h"

extern "C" {
__declspec(dllimport) long long __cdecl _Query_perf_frequency();
__declspec(dllimport) long long __cdecl _Query_perf_counter();
__declspec(dllimport) int __stdcall QueryPerformanceFrequency(long long*);
__declspec(dllimport) int __stdcall QueryPerformanceCounter(long long*);
__declspec(dllimport) void __stdcall Sleep(poly_u32);
}

static void test(poly::Test& t) {
    long long freq = _Query_perf_frequency();
    long long api = 0;
    QueryPerformanceFrequency(&api);
    t.check("the frequency is not zero", freq > 0, freq);
    t.expect("the frequency is QueryPerformanceFrequency's", freq, api);
    t.expect("it is the same every time", _Query_perf_frequency(), freq);
    long long before = 0, after = 0;
    QueryPerformanceCounter(&before);
    long long mid = _Query_perf_counter();
    QueryPerformanceCounter(&after);
    t.check("the counter lies between two QueryPerformanceCounter readings", before <= mid && mid <= after, mid - before);
    long long a = _Query_perf_counter();
    Sleep(30);
    long long b = _Query_perf_counter();
    long long ms = (b - a) * 1000 / freq;
    t.check("30 ms of sleep move the counter by 20 to 2000 ms", ms >= 20 && ms <= 2000, ms);
    long long prev = _Query_perf_counter();
    bool monotonic = true;
    for (int i = 0; i < 1000; i++) {
        long long now = _Query_perf_counter();
        if (now < prev) monotonic = false;
        prev = now;
    }
    t.check("the counter never goes back", monotonic, 0);
    t.value("frequency is a multiple of 1000", freq % 1000 == 0);
}
POLY_TEST_MAIN(test)
