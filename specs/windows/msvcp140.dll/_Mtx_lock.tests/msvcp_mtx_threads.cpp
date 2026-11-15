// poly-cxx: compiler=msvc
// msvcp140.dll _Mtx_* between threads: a second thread's trylock on a locked mutex is _Busy and its lock blocks until the
// owner unlocks; _Mtx_current_owns is per thread.
#include "poly_wtest.h"
#include "msvcp_threads.h"

extern "C" {
__declspec(dllimport) void* __stdcall CreateThread(void*, unsigned long long, unsigned long (__stdcall*)(void*), void*, poly_u32, poly_u32*);
__declspec(dllimport) poly_u32 __stdcall WaitForSingleObject(void*, poly_u32);
}

struct Shared {
    _Mtx_t m;
    int try_result;
    int owns_in_thread;
    volatile int got_lock;
    volatile int started;
    volatile int entered;
    volatile int tried;
};

static unsigned long __stdcall worker(void* p) {
    Shared* s = (Shared*)p;
    s->entered = 1;
    s->try_result = (int)_Mtx_trylock(s->m);
    s->tried = 1;
    s->owns_in_thread = _Mtx_current_owns(s->m);
    s->started = 1;
    _Mtx_lock(s->m);
    s->got_lock = 1;
    _Mtx_unlock(s->m);
    return 0;
}

static void test(poly::Test& t) {
    Shared s = {};
    t.expect("init", (int)_Mtx_init(&s.m, _Mtx_plain | _Mtx_try), 0);
    _Mtx_lock(s.m);
    void* h = CreateThread(0, 0, worker, &s, 0, 0);
    t.check("thread created", h != 0, 0);
    for (int i = 0; i < 100 && !s.started; i++) Sleep(10);
    t.expect("another thread's trylock is busy", s.try_result, (int)_Thrd_result::_Busy);
    t.expect("it does not own the mutex", s.owns_in_thread, 0);
    Sleep(50);
    t.expect("its lock is still blocked while we hold the mutex", s.got_lock, 0);
    t.expect("we still own it", _Mtx_current_owns(s.m), 1);
    _Mtx_unlock(s.m);
    t.expect("the thread ends", (int)WaitForSingleObject(h, 3000), 0);
    t.expect("it got the lock after the unlock", s.got_lock, 1);
    t.expect("and released it", s.m->_Count, 0);
    _Mtx_destroy(s.m);
}
POLY_TEST_MAIN(test)
