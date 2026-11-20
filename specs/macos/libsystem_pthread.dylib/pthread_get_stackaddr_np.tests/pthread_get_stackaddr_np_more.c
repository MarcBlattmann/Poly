/* pthread_get_stackaddr_np: the main thread's stack top is the same on
 * every call and from inside nested calls. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>

static __attribute__((noinline)) int nested(int depth, uintptr_t top, uintptr_t size) {
    volatile char pad[256];
    pad[0] = (char)depth;
    uintptr_t here = (uintptr_t)&pad[0];
    int ok = (uintptr_t)pthread_get_stackaddr_np(pthread_self()) == top && here < top && here >= top - size;
    if (depth > 0) ok &= nested(depth - 1, top, size);
    return ok + pad[0] - pad[0];
}

int test_main(void) {
    uintptr_t top = (uintptr_t)pthread_get_stackaddr_np(pthread_self());
    uintptr_t size = pthread_get_stacksize_np(pthread_self());
    check("the same value on a second call", (uintptr_t)pthread_get_stackaddr_np(pthread_self()) == top, 0);
    check("50 nested frames stay inside the stack and the value does not move", nested(50, top, size) == 1, 0);
    check("the stack size is stable too", pthread_get_stacksize_np(pthread_self()) == size, 0);
    return finish();
}
