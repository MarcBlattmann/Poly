/* pthread_create with attributes: detached, and an explicit stack size
 * that can really be used. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>

static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;
static int done;

static void *signal_done(void *arg) {
    (void)arg;
    pthread_mutex_lock(&m);
    done = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m);
    return NULL;
}

/* Uses about `depth` * 1 KiB of stack. */
static int __attribute__((noinline)) recurse(int depth) {
    volatile char buf[1024];
    buf[0] = (char)depth;
    buf[1023] = (char)depth;
    if (depth == 0) return buf[0] + buf[1023];
    return recurse(depth - 1) + buf[0];
}
static void *deep(void *arg) {
    recurse((int)(intptr_t)arg);
    return (void *)1; /* reached only when the recursion did not fault */
}

int test_main(void) {
    pthread_attr_t a;
    pthread_attr_init(&a);
    pthread_attr_setdetachstate(&a, PTHREAD_CREATE_DETACHED);
    pthread_t t;
    int r = pthread_create(&t, &a, signal_done, NULL);
    check("create detached returns 0", r == 0, r);
    pthread_mutex_lock(&m);
    while (!done) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    check("the detached thread ran", done == 1, done);
    pthread_attr_destroy(&a);

    /* A 4 MiB stack takes a 3000 KiB recursion; the default 512 KiB does not
     * need to (not tested: it would crash). */
    pthread_attr_init(&a);
    r = pthread_attr_setstacksize(&a, 4 << 20);
    check("setstacksize 4 MiB", r == 0, r);
    r = pthread_create(&t, &a, deep, (void *)3000);
    check("create with a 4 MiB stack returns 0", r == 0, r);
    void *rv = NULL;
    pthread_join(t, &rv);
    check("the 3000 KiB recursion completed", (intptr_t)rv == 1, (long long)(intptr_t)rv);
    pthread_attr_destroy(&a);

    pthread_attr_init(&a);
    pthread_attr_setstacksize(&a, 64 * 1024);
    r = pthread_create(&t, &a, deep, (void *)20);
    check("create with a 64 KiB stack returns 0", r == 0, r);
    pthread_join(t, &rv);
    check("a 20 KiB recursion completed on the 64 KiB stack", (intptr_t)rv == 1, (long long)(intptr_t)rv);
    pthread_attr_destroy(&a);

    pthread_attr_init(&a);
    pthread_attr_setdetachstate(&a, PTHREAD_CREATE_JOINABLE);
    r = pthread_create(&t, &a, signal_done, NULL);
    check("an explicitly joinable attr: join works", r == 0 && pthread_join(t, NULL) == 0, r);
    return finish();
}
