/* pthread_join: several threads joined in the reverse of their creation
 * order; a join waits at least as long as the thread runs. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>

static void *nap(void *arg) {
    usleep(20000 * (unsigned)(intptr_t)arg);
    return (void *)((intptr_t)arg * 10);
}

int test_main(void) {
    pthread_t t[5];
    for (int i = 0; i < 5; i++) pthread_create(&t[i], NULL, nap, (void *)(intptr_t)(i + 1));
    int bad = 0;
    for (int i = 4; i >= 0; i--) {
        void *rv = NULL;
        if (pthread_join(t[i], &rv) != 0 || (intptr_t)rv != (i + 1) * 10) bad++;
    }
    check("5 threads joined in reverse order, each with its own result", bad == 0, bad);

    struct timespec a, b;
    pthread_t s;
    clock_gettime(CLOCK_MONOTONIC, &a);
    pthread_create(&s, NULL, nap, (void *)5);
    pthread_join(s, NULL);
    clock_gettime(CLOCK_MONOTONIC, &b);
    long ms = (b.tv_sec - a.tv_sec) * 1000 + (b.tv_nsec - a.tv_nsec) / 1000000;
    check("the join took at least the 100 ms the thread slept", ms >= 99, ms);
    check("and not seconds", ms < 3000, ms);
    return finish();
}
