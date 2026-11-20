/* Mutex attributes and lifecycle: types, protocols and policies, destroy
 * of a locked mutex, use after destroy, the static initialiser, sizes. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>

static pthread_mutex_t gm = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t grm = PTHREAD_RECURSIVE_MUTEX_INITIALIZER;

int test_main(void) {
    pthread_mutexattr_t a;
    int v = -1;
    check("attr init", pthread_mutexattr_init(&a) == 0, 0);
    pthread_mutexattr_gettype(&a, &v);
    check("the default type is NORMAL (0)", v == PTHREAD_MUTEX_NORMAL && v == 0, v);
    value("PTHREAD_MUTEX_ERRORCHECK", PTHREAD_MUTEX_ERRORCHECK);
    value("PTHREAD_MUTEX_RECURSIVE", PTHREAD_MUTEX_RECURSIVE);
    value("PTHREAD_MUTEX_DEFAULT", PTHREAD_MUTEX_DEFAULT);
    check("settype 9: EINVAL (22)", pthread_mutexattr_settype(&a, 9) == EINVAL, 0);
    pthread_mutexattr_gettype(&a, &v);
    check("a failed settype leaves the type", v == 0, v);
    pthread_mutexattr_getprotocol(&a, &v);
    check("the default protocol is PTHREAD_PRIO_NONE (0)", v == PTHREAD_PRIO_NONE, v);
    check("setprotocol inherit: 0", pthread_mutexattr_setprotocol(&a, PTHREAD_PRIO_INHERIT) == 0, 0);
    check("setprotocol 99: EINVAL", pthread_mutexattr_setprotocol(&a, 99) == EINVAL, 0);
    pthread_mutexattr_getpolicy_np(&a, &v);
    check("the default policy is FIRSTFIT (3)", v == PTHREAD_MUTEX_POLICY_FIRSTFIT_NP && v == 3, v);
    check("setpolicy fairshare: 0", pthread_mutexattr_setpolicy_np(&a, PTHREAD_MUTEX_POLICY_FAIRSHARE_NP) == 0, 0);
    pthread_mutexattr_getpolicy_np(&a, &v);
    check("... reads back FAIRSHARE (1)", v == 1, v);
    check("setpolicy 77: EINVAL", pthread_mutexattr_setpolicy_np(&a, 77) == EINVAL, 0);
    check("attr destroy", pthread_mutexattr_destroy(&a) == 0, 0);

    check("a static initialiser works without init", pthread_mutex_trylock(&gm) == 0 && pthread_mutex_unlock(&gm) == 0, 0);
    check("the recursive static initialiser recurses", pthread_mutex_lock(&grm) == 0 && pthread_mutex_lock(&grm) == 0, 0);
    pthread_mutex_unlock(&grm);
    pthread_mutex_unlock(&grm);

    pthread_mutex_t m;
    pthread_mutex_init(&m, NULL);
    pthread_mutex_lock(&m);
    int r = pthread_mutex_destroy(&m);
    check("destroy of a locked mutex: EBUSY (16)", r == EBUSY, r);
    pthread_mutex_unlock(&m);
    check("destroy of an unlocked mutex: 0", pthread_mutex_destroy(&m) == 0, 0);
    r = pthread_mutex_lock(&m);
    check("lock after destroy: EINVAL", r == EINVAL, r);
    r = pthread_mutex_trylock(&m);
    check("trylock after destroy: EINVAL", r == EINVAL, r);
    value("sizeof(pthread_mutex_t)", sizeof(pthread_mutex_t));
    value("sizeof(pthread_mutexattr_t)", sizeof(pthread_mutexattr_t));
    value("sizeof(pthread_cond_t)", sizeof(pthread_cond_t));
    value("sizeof(pthread_attr_t)", sizeof(pthread_attr_t));
    value("sizeof(pthread_rwlock_t)", sizeof(pthread_rwlock_t));
    return finish();
}
