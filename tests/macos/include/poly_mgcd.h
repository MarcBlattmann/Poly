/*
 * Helpers for the libdispatch (GCD) tests: a test that includes this header
 * says `poly-mac: libsystem` in its header comment (see poly_mlibc.h) and
 * calls libdispatch through libSystem. Output goes through raw write calls.
 * Written for Poly from observation and public headers only.
 */
#ifndef POLY_MGCD_H
#define POLY_MGCD_H

#include "poly_mlibc.h"
#pragma clang diagnostic ignored "-Wunused-result"
#include <dispatch/dispatch.h>
#include <pthread.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static inline long long gcd_now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}
/* Wait for a semaphore for at most 10 seconds: 1 when it was signalled. */
static inline int gcd_wait(dispatch_semaphore_t s) {
    return dispatch_semaphore_wait(s, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC)) == 0;
}
static inline int gcd_on_main_thread(void) { return pthread_main_np() != 0; }
static inline long long gcd_current_thread_id(void) {
    uint64_t t = 0;
    pthread_threadid_np(NULL, &t);
    return (long long)t;
}
/* Let `ms` milliseconds pass (used only to let something that must NOT
 * happen have a chance to; never to order events that must happen). */
static inline void gcd_pause_ms(int ms) { usleep((useconds_t)ms * 1000); }

#endif
