/* pthread_get_stacksize_np queried from outside while the thread lives. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>

static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;
static int up, go;
static size_t inside;

static void *worker(void *arg) {
    (void)arg;
    pthread_mutex_lock(&m);
    inside = pthread_get_stacksize_np(pthread_self());
    up = 1;
    pthread_cond_broadcast(&c);
    while (!go) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    return NULL;
}

int test_main(void) {
    pthread_attr_t a;
    pthread_attr_init(&a);
    pthread_attr_setstacksize(&a, 256 * 1024);
    pthread_t t;
    pthread_create(&t, &a, worker, NULL);
    pthread_mutex_lock(&m);
    while (!up) pthread_cond_wait(&c, &m);
    size_t outside = pthread_get_stacksize_np(t);
    go = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m);
    pthread_join(t, NULL);
    check("outside and inside agree", outside == inside && outside != 0, (long long)outside);
    check("and cover the 256 KiB requested", outside >= 256 * 1024 && outside < 256 * 1024 + 65536, (long long)outside);
    return finish();
}
