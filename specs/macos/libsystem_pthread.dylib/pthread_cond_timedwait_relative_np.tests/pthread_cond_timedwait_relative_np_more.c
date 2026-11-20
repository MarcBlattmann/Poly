/* pthread_cond_timedwait_relative_np: repeated short timeouts add up;
 * a one-second and a nanosecond-sized timeout. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <time.h>

static long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

int test_main(void) {
    pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
    pthread_cond_t c = PTHREAD_COND_INITIALIZER;
    pthread_mutex_lock(&m);
    struct timespec ts = {0, 10 * 1000000};
    long long t0 = now_ns();
    int bad = 0;
    for (int i = 0; i < 10; i++) bad += pthread_cond_timedwait_relative_np(&c, &m, &ts) != ETIMEDOUT;
    long long dt = now_ns() - t0;
    check("ten 10 ms waits all time out", bad == 0, bad);
    check("together they took at least 100 ms", dt >= 100000000LL, dt / 1000000);

    ts.tv_sec = 1;
    ts.tv_nsec = 0;
    t0 = now_ns();
    int r = pthread_cond_timedwait_relative_np(&c, &m, &ts);
    dt = now_ns() - t0;
    check("one second: ETIMEDOUT", r == ETIMEDOUT, r);
    check("after at least 1000 ms", dt >= 1000000000LL && dt < 5000000000LL, dt / 1000000);

    ts.tv_sec = 0;
    ts.tv_nsec = 1;
    r = pthread_cond_timedwait_relative_np(&c, &m, &ts);
    check("one nanosecond: ETIMEDOUT", r == ETIMEDOUT, r);
    pthread_mutex_unlock(&m);
    return finish();
}
