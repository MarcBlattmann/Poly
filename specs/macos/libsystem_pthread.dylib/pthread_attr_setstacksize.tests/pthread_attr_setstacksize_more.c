/* pthread_attr_setstacksize: an attr copied into several threads, the
 * detach state and the stack size independent of each other. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <limits.h>
#include <stdint.h>

int test_main(void) {
    pthread_attr_t a;
    pthread_attr_init(&a);
    int ds = 0;
    size_t sz = 0;
    pthread_attr_getdetachstate(&a, &ds);
    check("default detach state is joinable (1)", ds == PTHREAD_CREATE_JOINABLE && ds == 1, ds);
    pthread_attr_setdetachstate(&a, PTHREAD_CREATE_DETACHED);
    pthread_attr_setstacksize(&a, 1 << 20);
    pthread_attr_getdetachstate(&a, &ds);
    pthread_attr_getstacksize(&a, &sz);
    check("setting the stack size keeps the detach state (2)", ds == PTHREAD_CREATE_DETACHED && ds == 2, ds);
    check("and holds 1 MiB", sz == (1 << 20), (long long)sz);
    pthread_attr_setdetachstate(&a, PTHREAD_CREATE_JOINABLE);
    pthread_attr_getstacksize(&a, &sz);
    check("setting the detach state keeps the stack size", sz == (1 << 20), (long long)sz);
    check("a bad detach state: EINVAL", pthread_attr_setdetachstate(&a, 7) == 22, 0);
    pthread_attr_t b;
    pthread_attr_init(&b);
    pthread_attr_getstacksize(&b, &sz);
    check("a second attr starts at the default", sz == 524288, (long long)sz);
    value("PTHREAD_STACK_MIN (<limits.h>; 16384 on arm64, 8192 on x86_64)", PTHREAD_STACK_MIN);
    return finish();
}
