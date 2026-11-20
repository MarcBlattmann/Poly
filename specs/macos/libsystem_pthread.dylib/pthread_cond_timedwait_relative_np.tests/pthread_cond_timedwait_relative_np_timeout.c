/* pthread_cond_timedwait_relative_np: waits at most the given relative
 * time; ETIMEDOUT (60); the mutex is held again on return. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <time.h>

static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;

static long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

int test_main(void) {
    struct timespec ts = {0, 50 * 1000000};
    pthread_mutex_lock(&m);
    long long t0 = now_ns();
    int r = pthread_cond_timedwait_relative_np(&c, &m, &ts);
    long long dt = now_ns() - t0;
    check("50 ms with nobody signalling: ETIMEDOUT (60)", r == ETIMEDOUT && r == 60, r);
    check("it waited at least 50 ms", dt >= 50000000LL, dt / 1000000);
    check("and less than 2 s", dt < 2000000000LL, dt / 1000000);
    check("the mutex is held again (trylock: EBUSY)", pthread_mutex_trylock(&m) == EBUSY, 0);

    ts.tv_sec = 0;
    ts.tv_nsec = 0;
    t0 = now_ns();
    r = pthread_cond_timedwait_relative_np(&c, &m, &ts);
    check("zero time: ETIMEDOUT", r == ETIMEDOUT, r);
    check("zero time returns at once (< 500 ms)", now_ns() - t0 < 500000000LL, (now_ns() - t0) / 1000000);

    ts.tv_sec = 0;
    ts.tv_nsec = 1000000000;
    r = pthread_cond_timedwait_relative_np(&c, &m, &ts);
    check("tv_nsec == 1e9: EINVAL (22)", r == EINVAL, r);
    ts.tv_nsec = -1;
    r = pthread_cond_timedwait_relative_np(&c, &m, &ts);
    check("tv_nsec == -1: EINVAL", r == EINVAL, r);
    check("the mutex is still held after the errors", pthread_mutex_trylock(&m) == EBUSY, 0);
    pthread_mutex_unlock(&m);
    value("ETIMEDOUT", ETIMEDOUT);
    return finish();
}
