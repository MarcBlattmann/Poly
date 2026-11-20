/* pthread_get_stackaddr_np: the address one past the highest byte of the
 * thread's stack (the stack grows down from it). */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>
#include <unistd.h>

static int in_thread_ok, in_thread_page_aligned;
static uintptr_t thread_top;

/* An address inside the caller's frame. */
#define local_address() ((uintptr_t)__builtin_frame_address(0))

static void *worker(void *arg) {
    (void)arg;
    pthread_t me = pthread_self();
    uintptr_t top = (uintptr_t)pthread_get_stackaddr_np(me);
    uintptr_t size = pthread_get_stacksize_np(me);
    uintptr_t here = local_address();
    in_thread_ok = here < top && here >= top - size && top - here < 16384;
    in_thread_page_aligned = top % 4096 == 0;
    thread_top = top;
    return NULL;
}

int test_main(void) {
    uintptr_t top = (uintptr_t)pthread_get_stackaddr_np(pthread_self());
    uintptr_t size = pthread_get_stacksize_np(pthread_self());
    uintptr_t here = local_address();
    check("main thread: a local is below the address", here < top, 0);
    check("main thread: the local is within the stack size", here >= top - size, (long long)(top - here));
    check("main thread: the main function's frame is near the top (< 1 MiB)", top - here < (1 << 20), (long long)(top - here));
    check("main thread: the address is page aligned", top % 4096 == 0, (long long)(top % 4096));

    pthread_t t;
    pthread_create(&t, NULL, worker, NULL);
    pthread_join(t, NULL);
    check("a thread: its local is inside [top - size, top) and near the top", in_thread_ok, 0);
    check("a thread: the address is page aligned", in_thread_page_aligned, 0);
    check("the thread's stack is not the main thread's", thread_top != top, 0);
    check("the two stacks do not overlap", thread_top <= top - size || thread_top - 512 * 1024 >= top, 0);
    return finish();
}
