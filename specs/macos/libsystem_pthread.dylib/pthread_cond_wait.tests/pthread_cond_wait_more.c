/* pthread_cond_wait: init / destroy, and a condition variable shared by
 * two mutexes in turn. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>

static pthread_cond_t c;
static pthread_mutex_t m1 = PTHREAD_MUTEX_INITIALIZER, m2 = PTHREAD_MUTEX_INITIALIZER;
static int phase, stage;

static void *waiter(void *arg) {
    pthread_mutex_t *m = arg;
    pthread_mutex_lock(m);
    while (phase == 0) pthread_cond_wait(&c, m);
    stage++;
    pthread_mutex_unlock(m);
    return NULL;
}

int test_main(void) {
    int r = pthread_cond_init(&c, NULL);
    check("init returns 0", r == 0, r);
    pthread_t t;
    pthread_create(&t, NULL, waiter, &m1);
    pthread_mutex_lock(&m1);
    phase = 1;
    pthread_cond_signal(&c);
    pthread_mutex_unlock(&m1);
    pthread_join(t, NULL);
    check("a waiter on m1 was woken", stage == 1, stage);

    phase = 0;
    pthread_create(&t, NULL, waiter, &m2); /* the same condition, another mutex, later */
    pthread_mutex_lock(&m2);
    phase = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m2);
    pthread_join(t, NULL);
    check("then a waiter on m2 was woken", stage == 2, stage);
    check("destroy returns 0", pthread_cond_destroy(&c) == 0, 0);
    return finish();
}
