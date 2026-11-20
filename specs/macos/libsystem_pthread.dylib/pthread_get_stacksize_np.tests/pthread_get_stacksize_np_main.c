/* pthread_get_stacksize_np of the main thread: it follows RLIMIT_STACK. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>
#include <sys/resource.h>

int test_main(void) {
    size_t ss = pthread_get_stacksize_np(pthread_self());
    struct rlimit rl;
    getrlimit(RLIMIT_STACK, &rl);
    value("main thread stack size", (long long)ss);
    value("RLIMIT_STACK soft limit", (long long)rl.rlim_cur);
    check("the main thread's stack size is the stack limit", ss == rl.rlim_cur, (long long)ss);
    check("it is 8 MiB less 16 KiB", ss == 8 * 1024 * 1024 - 16384, (long long)ss);
    check("a multiple of 4096", ss % 4096 == 0, (long long)(ss % 4096));
    return finish();
}
