/* pthread_setspecific: 100 threads each keep their own value across many
 * other threads' sets. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <sched.h>
#include <stdint.h>

static pthread_key_t key;
static int wrong;

static void *worker(void *arg) {
    intptr_t me = (intptr_t)arg;
    pthread_setspecific(key, (void *)me);
    for (int i = 0; i < 200; i++) {
        sched_yield();
        if ((intptr_t)pthread_getspecific(key) != me) __atomic_fetch_add(&wrong, 1, __ATOMIC_RELAXED);
    }
    return NULL;
}

int test_main(void) {
    pthread_key_create(&key, NULL);
    pthread_t t[100];
    for (int i = 0; i < 100; i++) pthread_create(&t[i], NULL, worker, (void *)(intptr_t)(i + 1));
    for (int i = 0; i < 100; i++) pthread_join(t[i], NULL);
    check("no thread ever read another's value", wrong == 0, wrong);
    return finish();
}
