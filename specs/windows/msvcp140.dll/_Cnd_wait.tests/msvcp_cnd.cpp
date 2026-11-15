// poly-cxx: compiler=msvc
// msvcp140.dll _Cnd_init, _Cnd_wait, _Cnd_signal, _Cnd_broadcast, _Cnd_destroy: a waiter releases the mutex while it waits and
// holds it again when it returns; signal wakes one waiter, broadcast all; the object is _Cnd_internal_imp_t (72 bytes).
#include "poly_wtest.h"
#include "msvcp_threads.h"

extern "C" {
__declspec(dllimport) void* __stdcall CreateThread(void*, unsigned long long, unsigned long (__stdcall*)(void*), void*, poly_u32, poly_u32*);
__declspec(dllimport) poly_u32 __stdcall WaitForSingleObject(void*, poly_u32);
}

struct Shared {
    _Mtx_t m;
    _Cnd_t c;
    int go;
    int waiting;
    int woken;
    int owned_after_wait;
};

static unsigned long __stdcall waiter(void* p) {
    Shared* s = (Shared*)p;
    _Mtx_lock(s->m);
    s->waiting++;
    while (!s->go) _Cnd_wait(s->c, s->m);
    s->owned_after_wait += _Mtx_current_owns(s->m);
    s->woken++;
    _Mtx_unlock(s->m);
    return 0;
}

static int waiting_count(Shared& s) {
    _Mtx_lock(s.m);
    int n = s.waiting;
    _Mtx_unlock(s.m);
    return n;
}

static void test(poly::Test& t) {
    t.expect("sizeof(_Cnd_internal_imp_t)", (long long)sizeof(_Cnd_internal_imp_t), 72);
    Shared s = {};
    t.expect("_Mtx_init", (int)_Mtx_init(&s.m, _Mtx_plain), 0);
    t.expect("_Cnd_init", (int)_Cnd_init(&s.c), 0);
    t.check("the condition variable object", s.c != nullptr, 0);
    void* h[3];
    for (int i = 0; i < 3; i++) h[i] = CreateThread(0, 0, waiter, &s, 0, 0);
    while (waiting_count(s) < 3) Sleep(1);
    Sleep(30);
    t.expect("the waiters released the mutex: we can lock it", (int)_Mtx_trylock(s.m), 0);
    t.expect("nobody woke without a signal", s.woken, 0);
    s.go = 1;
    _Mtx_unlock(s.m);
    t.expect("_Cnd_signal succeeds", (int)_Cnd_signal(s.c), 0);
    Sleep(100);
    _Mtx_lock(s.m);
    int after_signal = s.woken;
    _Mtx_unlock(s.m);
    t.check("a signal wakes at least one waiter", after_signal >= 1, after_signal);
    t.expect("_Cnd_broadcast succeeds", (int)_Cnd_broadcast(s.c), 0);
    for (int i = 0; i < 3; i++) t.expect("waiter ends", (int)WaitForSingleObject(h[i], 5000), 0);
    t.expect("all three woke", s.woken, 3);
    t.expect("each held the mutex when _Cnd_wait returned", s.owned_after_wait, 3);
    _Cnd_destroy(s.c);
    _Mtx_destroy(s.m);
}
POLY_TEST_MAIN(test)
