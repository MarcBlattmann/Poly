// poly-cxx: compiler=msvc
// msvcp140.dll _Thrd_create, _Thrd_join, _Thrd_detach, _Thrd_current, _Thrd_equal, _Thrd_id, _Thrd_yield,
// _Thrd_sleep, _Thrd_hardware_concurrency: _Thrd_t is {HANDLE, thread id}; a thread's start function returns an
// int that _Thrd_join hands back; _Thrd_sleep takes an absolute xtime.
#include "poly_wtest.h"
#include "msvcp_threads.h"

static volatile unsigned g_thread_id;
static volatile int g_ran;

static int __cdecl start(void* arg) {
    g_thread_id = GetCurrentThreadId();
    g_ran = (int)(long long)arg;
    return 1234;
}

static void test(poly::Test& t) {
    t.expect("sizeof(_Thrd_t)", (long long)sizeof(_Thrd_t), 16);
    _Thrd_t th = {};
    t.expect("_Thrd_create succeeds", (int)_Thrd_create(&th, start, (void*)77), 0);
    t.check("a handle is returned", th._Hnd != nullptr, 0);
    int result = 0;
    t.expect("_Thrd_join succeeds", (int)_Thrd_join(th, &result), 0);
    t.expect("the start function's return value", result, 1234);
    t.expect("the argument arrived", g_ran, 77);
    t.expect("_Thrd_t::_Id is the thread's id", (long long)th._Id, (long long)g_thread_id);
    t.check("it is not this thread", g_thread_id != GetCurrentThreadId(), 0);

    _Thrd_t me = _Thrd_current();
    t.expect("_Thrd_current's id", (long long)me._Id, (long long)GetCurrentThreadId());
    t.expect("_Thrd_id", (long long)_Thrd_id(), (long long)GetCurrentThreadId());
    t.expect("_Thrd_equal of the same thread", _Thrd_equal(me, _Thrd_current()), 1);
    t.expect("_Thrd_equal of different threads", _Thrd_equal(me, th), 0);

    _Thrd_t d = {};
    g_ran = 0;
    t.expect("create to detach", (int)_Thrd_create(&d, start, (void*)5), 0);
    t.expect("_Thrd_detach succeeds", (int)_Thrd_detach(d), 0);
    for (int i = 0; i < 200 && g_ran != 5; i++) Sleep(10);
    t.expect("the detached thread ran", g_ran, 5);

    _Thrd_yield();
    t.check("_Thrd_hardware_concurrency is at least 1", _Thrd_hardware_concurrency() >= 1, _Thrd_hardware_concurrency());

    long long now = _Xtime_get_ticks();
    long long until = now + 500000;   // 50 ms
    poly_xtime xt = {until / 10000000, (long)((until % 10000000) * 100)};
    _Thrd_sleep(&xt);
    long long waited = (_Xtime_get_ticks() - now) / 10000;
    t.check("_Thrd_sleep(absolute time) waits until that time", waited >= 40 && waited < 2000, waited);
    poly_xtime past = {0, 0};
    long long t0 = _Xtime_get_ticks();
    _Thrd_sleep(&past);
    t.check("a time in the past does not wait", (_Xtime_get_ticks() - t0) / 10000 < 500, 0);
}
POLY_TEST_MAIN(test)
