/* pthread_join: waits for the thread, delivers its result (from return or
 * pthread_exit), accepts NULL for the result; a second join fails. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <unistd.h>

static void *slow(void *arg) {
    usleep(50000);
    return arg;
}
static void *exiter(void *arg) {
    pthread_exit((void *)((intptr_t)arg + 1));
    return (void *)99; /* not reached */
}

int test_main(void) {
    pthread_t t;
    void *rv = (void *)-1;
    pthread_create(&t, NULL, slow, (void *)7);
    int r = pthread_join(t, &rv);
    check("join of a still running thread returns 0", r == 0, r);
    check("with its return value", (intptr_t)rv == 7, (long long)(intptr_t)rv);
    r = pthread_join(t, &rv);
    check("a second join: ESRCH (3)", r == ESRCH && r == 3, r);
    check("a failed join leaves the result alone", (intptr_t)rv == 7, (long long)(intptr_t)rv);

    pthread_create(&t, NULL, exiter, (void *)1);
    pthread_join(t, &rv);
    check("pthread_exit's value is the result (2), the return after it never ran", (intptr_t)rv == 2, (long long)(intptr_t)rv);

    pthread_create(&t, NULL, slow, NULL);
    r = pthread_join(t, NULL);
    check("a NULL result pointer is accepted", r == 0, r);

    pthread_create(&t, NULL, slow, (void *)3);
    usleep(150000); /* long finished before the join */
    rv = NULL;
    r = pthread_join(t, &rv);
    check("join of an already finished thread works", r == 0 && (intptr_t)rv == 3, r);
    return finish();
}
