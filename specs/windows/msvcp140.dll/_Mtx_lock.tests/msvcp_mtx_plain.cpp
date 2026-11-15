// poly-cxx: compiler=msvc
// msvcp140.dll _Mtx_init, _Mtx_lock, _Mtx_trylock, _Mtx_unlock, _Mtx_current_owns, _Mtx_destroy on a plain mutex: the
// object the DLL allocates has the header's layout (_Mtx_internal_imp_t: int type at 0, the critical section
// storage at 8, the owner's thread id at 72, the lock count at 76, 80 bytes): the owner and the count are
// visible to the caller; results are the _Thrd_result codes (_Success 0, _Nomem 1, _Timedout 2, _Busy 3, _Error 4).
#include "poly_wtest.h"
#include "msvcp_threads.h"

static void test(poly::Test& t) {
    t.expect("sizeof(_Mtx_internal_imp_t)", (long long)sizeof(_Mtx_internal_imp_t), 80);
    t.expect("offsetof _Thread_id", (long long)__builtin_offsetof(_Mtx_internal_imp_t, _Thread_id), 72);
    t.expect("offsetof _Count", (long long)__builtin_offsetof(_Mtx_internal_imp_t, _Count), 76);
    _Mtx_t m = nullptr;
    t.expect("_Mtx_init succeeds", (int)_Mtx_init(&m, _Mtx_plain), 0);
    t.check("the mutex object is returned", m != nullptr, 0);
    t.expect("the type is stored", m->_Type, _Mtx_plain);
    t.expect("not owned: thread id -1", m->_Thread_id, -1);
    t.expect("not owned: count 0", m->_Count, 0);
    t.expect("_Mtx_current_owns before locking", _Mtx_current_owns(m), 0);
    t.expect("_Mtx_lock succeeds", (int)_Mtx_lock(m), 0);
    t.expect("the owner is this thread", (long long)m->_Thread_id, (long long)GetCurrentThreadId());
    t.expect("the count is 1", m->_Count, 1);
    t.expect("_Mtx_current_owns while locked", _Mtx_current_owns(m), 1);
    t.expect("_Mtx_unlock succeeds", (int)_Mtx_unlock(m), 0);
    t.expect("unlocked: thread id -1 again", m->_Thread_id, -1);
    t.expect("unlocked: count 0", m->_Count, 0);
    t.expect("_Mtx_current_owns after unlocking", _Mtx_current_owns(m), 0);
    t.expect("trylock on a free mutex succeeds", (int)_Mtx_trylock(m), 0);
    t.expect("and owns it", _Mtx_current_owns(m), 1);
    t.expect("the count is 1", m->_Count, 1);
    t.expect("unlock", (int)_Mtx_unlock(m), 0);
    _Mtx_destroy(m);
    _Mtx_t other = nullptr;
    t.expect("a second mutex", (int)_Mtx_init(&other, _Mtx_plain | _Mtx_try), 0);
    t.expect("its type", other->_Type, _Mtx_plain | _Mtx_try);
    _Mtx_destroy(other);
}
POLY_TEST_MAIN(test)
