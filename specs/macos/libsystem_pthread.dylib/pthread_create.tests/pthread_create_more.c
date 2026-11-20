/* pthread_create: a thread creates a thread; the creator may join its
 * creator's child; the created thread outlives its creating thread. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>
#include <unistd.h>

static pthread_t grandchild;
static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;
static int grandchild_ready, release, grandchild_done;

static void *grandchild_fn(void *arg) {
    pthread_mutex_lock(&m);
    while (!release) pthread_cond_wait(&c, &m);
    grandchild_done = 1;
    pthread_mutex_unlock(&m);
    return arg;
}
static void *child_fn(void *arg) {
    (void)arg;
    pthread_create(&grandchild, NULL, grandchild_fn, (void *)5);
    pthread_mutex_lock(&m);
    grandchild_ready = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m);
    return NULL; /* the child ends while the grandchild still waits */
}

int test_main(void) {
    pthread_t child;
    check("create the child", pthread_create(&child, NULL, child_fn, NULL) == 0, 0);
    pthread_mutex_lock(&m);
    while (!grandchild_ready) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    check("join the child", pthread_join(child, NULL) == 0, 0);
    /* The child is gone; its child still runs and can be joined by the
     * main thread, which did not create it. */
    pthread_mutex_lock(&m);
    release = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m);
    void *rv = NULL;
    int r = pthread_join(grandchild, &rv);
    check("the main thread joins a grandchild", r == 0, r);
    check("with its result", (intptr_t)rv == 5 && grandchild_done, (long long)(intptr_t)rv);
    return finish();
}
