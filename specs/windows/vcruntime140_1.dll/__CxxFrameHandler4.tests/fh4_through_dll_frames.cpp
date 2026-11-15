// poly-cxx: compiler=msvc cl=/EHs
// __CxxFrameHandler4: an exception thrown in a callback that a DLL function (ucrtbase's qsort_s)
// called passes through the DLL's frames (which have no C++ handler: the search and the unwind go
// on through them) to the catch clause of the caller; destructors on both sides of the DLL
// frames run once, in order; the DLL's data is not touched by the unwind.
#include "poly_wtest.h"

extern "C" __declspec(dllimport) void __cdecl qsort_s(void* base, unsigned long long num, unsigned long long width,
                                                      int(__cdecl* compare)(void*, const void*, const void*),
                                                      void* context);

static poly::Log g_log;

struct D {
    const char* n;
    D(const char* n) : n(n) {}
    ~D() { g_log.add(n); }
};

struct Ctx {
    int calls;
    int throw_at;
};

static int __cdecl cmp(void* context, const void* a, const void* b) {
    Ctx* c = (Ctx*)context;
    D d("~cmp");
    if (++c->calls == c->throw_at) throw *(const int*)a;
    return *(const int*)a - *(const int*)b;
}

__declspec(noinline) static int sort_and_catch(int* v, int n, Ctx* c) {
    D d("~caller");
    try {
        qsort_s(v, n, sizeof(int), cmp, c);
    } catch (int e) {
        g_log.add("catch");
        return e;
    }
    return -1;
}

static void test(poly::Test& t) {
    int v[8] = {5, 3, 9, 1, 7, 2, 8, 4};
    Ctx c = {0, 3};
    int e = sort_and_catch(v, 8, &c);
    t.check("an exception from the callback reaches the caller", e != -1, e);
    t.check("it was thrown by the third comparison", c.calls == 3, c.calls);
    t.text("log", g_log.str());
    g_log.clear();
    Ctx c2 = {0, 1000};
    int w[5] = {5, 4, 3, 2, 1};
    t.expect("no exception without a throw", sort_and_catch(w, 5, &c2), -1);
    t.expect("sorted", w[0] * 10000 + w[1] * 1000 + w[2] * 100 + w[3] * 10 + w[4], 12345);
}
POLY_TEST_MAIN(test)
