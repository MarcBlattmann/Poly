// poly-cxx: compiler=msvc
// msvcp140.dll _Mtx_* on a recursive mutex (_Mtx_recursive | _Mtx_plain): the same thread may lock again; the lock
// count rises and falls; the owner is cleared only when the count reaches 0.
#include "poly_wtest.h"
#include "msvcp_threads.h"

static void test(poly::Test& t) {
    _Mtx_t m = nullptr;
    t.expect("init recursive", (int)_Mtx_init(&m, _Mtx_recursive | _Mtx_plain), 0);
    t.expect("type", m->_Type, _Mtx_recursive | _Mtx_plain);
    t.expect("lock 1", (int)_Mtx_lock(m), 0);
    t.expect("lock 2", (int)_Mtx_lock(m), 0);
    t.expect("count 2", m->_Count, 2);
    t.expect("trylock succeeds for the owner", (int)_Mtx_trylock(m), 0);
    t.expect("count 3", m->_Count, 3);
    t.expect("owner", (long long)m->_Thread_id, (long long)GetCurrentThreadId());
    t.expect("unlock to 2", (int)_Mtx_unlock(m), 0);
    t.expect("count 2 again", m->_Count, 2);
    t.expect("still owned by this thread", _Mtx_current_owns(m), 1);
    t.expect("unlock to 1", (int)_Mtx_unlock(m), 0);
    t.expect("owner still set at count 1", (long long)m->_Thread_id, (long long)GetCurrentThreadId());
    t.expect("unlock to 0", (int)_Mtx_unlock(m), 0);
    t.expect("owner cleared (thread id -1)", m->_Thread_id, -1);
    t.expect("count 0", m->_Count, 0);
    t.expect("not owned", _Mtx_current_owns(m), 0);
    _Mtx_destroy(m);
}
POLY_TEST_MAIN(test)
