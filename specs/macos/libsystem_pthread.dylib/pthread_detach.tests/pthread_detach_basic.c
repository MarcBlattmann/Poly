/* pthread_detach: a detached thread cannot be joined or detached again;
 * after it ended its id is gone (ESRCH). */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <unistd.h>

static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;
static int started, release, finished;

static void *hold(void *arg) {
    (void)arg;
    pthread_mutex_lock(&m);
    started = 1;
    pthread_cond_broadcast(&c);
    while (!release) pthread_cond_wait(&c, &m);
    finished = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m);
    return NULL;
}

int test_main(void) {
    pthread_t t;
    pthread_create(&t, NULL, hold, NULL);
    pthread_mutex_lock(&m);
    while (!started) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);

    int r = pthread_detach(t);
    check("detach of a running thread returns 0", r == 0, r);
    r = pthread_detach(t);
    check("a second detach: EINVAL (22)", r == EINVAL, r);
    r = pthread_join(t, NULL);
    check("join after detach: EINVAL", r == EINVAL, r);

    pthread_mutex_lock(&m);
    release = 1;
    pthread_cond_broadcast(&c);
    while (!finished) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    usleep(100000); /* the thread leaves */
    r = pthread_detach(t);
    check("detach of an ended detached thread: ESRCH (3)", r == ESRCH, r);
    r = pthread_join(t, NULL);
    check("join of an ended detached thread: ESRCH", r == ESRCH, r);
    return finish();
}
