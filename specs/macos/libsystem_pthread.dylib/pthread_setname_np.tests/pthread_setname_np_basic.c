/* pthread_setname_np names the calling thread; pthread_getname_np reads
 * the name of any thread into a buffer. A new thread starts without a
 * name and does not inherit its creator's. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <string.h>
#include <unistd.h>

static char seen_initial[64], seen_after[64];
static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;
static int named, release;

static void *worker(void *arg) {
    (void)arg;
    pthread_getname_np(pthread_self(), seen_initial, sizeof seen_initial);
    int r = pthread_setname_np("worker-1");
    pthread_getname_np(pthread_self(), seen_after, sizeof seen_after);
    pthread_mutex_lock(&m);
    named = r == 0 ? 1 : -1;
    pthread_cond_broadcast(&c);
    while (!release) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    return NULL;
}

int test_main(void) {
    char buf[64];
    memset(buf, 'x', sizeof buf);
    int r = pthread_getname_np(pthread_self(), buf, sizeof buf);
    check("getname of the main thread returns 0", r == 0, r);
    check_str("the main thread has no name at first", buf, "");
    r = pthread_setname_np("main-thread");
    check("setname returns 0", r == 0, r);
    pthread_getname_np(pthread_self(), buf, sizeof buf);
    check_str("the name reads back", buf, "main-thread");

    pthread_t t;
    pthread_create(&t, NULL, worker, NULL);
    pthread_mutex_lock(&m);
    while (!named) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    check("setname inside a thread returns 0", named == 1, named);
    check_str("a new thread starts without a name (not main's)", seen_initial, "");
    check_str("the thread's own name after setname", seen_after, "worker-1");
    r = pthread_getname_np(t, buf, sizeof buf);
    check("getname of another, running thread returns 0", r == 0, r);
    check_str("it reads that thread's name", buf, "worker-1");
    pthread_getname_np(pthread_self(), buf, sizeof buf);
    check_str("main's name is unchanged by the other thread's setname", buf, "main-thread");
    pthread_mutex_lock(&m);
    release = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m);
    pthread_join(t, NULL);
    r = pthread_getname_np(t, buf, sizeof buf);
    check("getname of a joined thread: ESRCH (3)", r == ESRCH, r);

    r = pthread_setname_np("");
    pthread_getname_np(pthread_self(), buf, sizeof buf);
    check("an empty name is accepted and clears it", r == 0 && buf[0] == 0, r);
    r = pthread_setname_np("a b\tc\xc3\xa9");
    pthread_getname_np(pthread_self(), buf, sizeof buf);
    check_str("any bytes are kept: spaces, a tab, UTF-8", buf, "a b\tc\xc3\xa9");
    return finish();
}
