/* pthread_threadid_np: the system-wide id of a thread: NULL means the
 * calling thread; the id of another thread can be read; EINVAL without an
 * output; ESRCH for a thread that is gone. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <sys/syscall.h>
#include <unistd.h>

static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;
static uint64_t own_id;
static int published, release;

static void *worker(void *arg) {
    (void)arg;
    pthread_threadid_np(NULL, &own_id);
    pthread_mutex_lock(&m);
    published = 1;
    pthread_cond_broadcast(&c);
    while (!release) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    return NULL;
}

int test_main(void) {
    uint64_t me = 0, me2 = 0;
    int r = pthread_threadid_np(NULL, &me);
    check("NULL thread: returns 0", r == 0, r);
    check("the id is nonzero", me != 0, (long long)me);
    r = pthread_threadid_np(pthread_self(), &me2);
    check("pthread_self(): returns 0", r == 0, r);
    check("and gives the same id", me2 == me, (long long)me2);
    check("it equals the thread_selfid system call's result", (uint64_t)syscall(SYS_thread_selfid) == me, 0);
    check("the main thread's id is not the process id", me != (uint64_t)getpid(), 0);

    r = pthread_threadid_np(NULL, NULL);
    check("NULL output with the NULL thread: EINVAL (22)", r == EINVAL && r == 22, r);
    r = pthread_threadid_np(pthread_self(), NULL);
    check("NULL output with self: EINVAL", r == EINVAL, r);

    pthread_t t;
    pthread_create(&t, NULL, worker, NULL);
    pthread_mutex_lock(&m);
    while (!published) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    uint64_t other = 0;
    r = pthread_threadid_np(t, &other);
    check("another thread's id can be read: returns 0", r == 0, r);
    check("it is the id that thread reads for itself", other == own_id && other != 0, (long long)other);
    check("and differs from main's", other != me, 0);
    pthread_mutex_lock(&m);
    release = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m);
    pthread_join(t, NULL);
    other = 12345;
    r = pthread_threadid_np(t, &other);
    check("a joined thread: ESRCH (3)", r == ESRCH, r);
    return finish();
}
