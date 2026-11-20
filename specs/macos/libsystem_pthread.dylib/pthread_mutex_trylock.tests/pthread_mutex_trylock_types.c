/* pthread_mutex_lock / trylock / unlock for the three mutex types: what
 * the owner and other threads get. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>

static int tr, other_unlock;
static void *try_it(void *m) {
    tr = pthread_mutex_trylock(m);
    if (tr == 0) pthread_mutex_unlock(m);
    return NULL;
}
static void *unlock_it(void *m) {
    other_unlock = pthread_mutex_unlock(m);
    return NULL;
}
static int try_other(pthread_mutex_t *m) {
    pthread_t t;
    pthread_create(&t, NULL, try_it, m);
    pthread_join(t, NULL);
    return tr;
}

int test_main(void) {
    pthread_mutex_t m;
    pthread_mutexattr_t a;
    pthread_t t;
    int r;
    /* The default type. */
    pthread_mutex_init(&m, NULL);
    check("free: trylock returns 0", pthread_mutex_trylock(&m) == 0, 0);
    r = pthread_mutex_trylock(&m);
    check("default type, owner trylocks again: EBUSY (16)", r == EBUSY && r == 16, r);
    r = try_other(&m);
    check("another thread's trylock: EBUSY", r == EBUSY, r);
    check("unlock", pthread_mutex_unlock(&m) == 0, 0);
    check("unlock of an unlocked default mutex: 0 (no error)", pthread_mutex_unlock(&m) == 0, 0);
    pthread_mutex_lock(&m);
    pthread_create(&t, NULL, unlock_it, &m);
    pthread_join(t, NULL);
    check("default type: another thread may unlock it (0)", other_unlock == 0, other_unlock);
    r = try_other(&m);
    check("... and it is free again", r == 0, r);
    pthread_mutex_destroy(&m);

    /* Error checking. */
    pthread_mutexattr_init(&a);
    check("settype errorcheck", pthread_mutexattr_settype(&a, PTHREAD_MUTEX_ERRORCHECK) == 0, 0);
    pthread_mutex_init(&m, &a);
    pthread_mutex_lock(&m);
    r = pthread_mutex_lock(&m);
    check("errorcheck: relock by the owner: EDEADLK (11)", r == EDEADLK && r == 11, r);
    r = pthread_mutex_trylock(&m);
    check("errorcheck: trylock by the owner: EBUSY", r == EBUSY, r);
    pthread_create(&t, NULL, unlock_it, &m);
    pthread_join(t, NULL);
    check("errorcheck: unlock by another thread: EPERM (1)", other_unlock == EPERM && other_unlock == 1, other_unlock);
    check("errorcheck: unlock by the owner", pthread_mutex_unlock(&m) == 0, 0);
    r = pthread_mutex_unlock(&m);
    check("errorcheck: unlock of an unlocked mutex: EPERM", r == EPERM, r);
    pthread_mutex_destroy(&m);

    /* Recursive. */
    pthread_mutexattr_settype(&a, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(&m, &a);
    check("recursive: lock", pthread_mutex_lock(&m) == 0, 0);
    check("recursive: relock by the owner", pthread_mutex_lock(&m) == 0, 0);
    check("recursive: trylock by the owner", pthread_mutex_trylock(&m) == 0, 0);
    r = try_other(&m);
    check("recursive: another thread's trylock: EBUSY", r == EBUSY, r);
    pthread_mutex_unlock(&m);
    pthread_mutex_unlock(&m);
    r = try_other(&m);
    check("recursive: still held after two of three unlocks", r == EBUSY, r);
    check("recursive: third unlock", pthread_mutex_unlock(&m) == 0, 0);
    r = try_other(&m);
    check("recursive: free after as many unlocks as locks", r == 0, r);
    r = pthread_mutex_unlock(&m);
    check("recursive: one unlock too many: EPERM", r == EPERM, r);
    pthread_mutex_destroy(&m);
    pthread_mutexattr_destroy(&a);
    return finish();
}
