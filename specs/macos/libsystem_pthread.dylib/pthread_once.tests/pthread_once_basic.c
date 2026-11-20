/* pthread_once: the routine runs the first time only, whichever thread
 * calls; every caller returns after it has completed. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>
#include <unistd.h>

static pthread_once_t once = PTHREAD_ONCE_INIT;
static int runs, finished_before_return_bad;
static int init_done;

static void init(void) {
    usleep(30000); /* a slow initialiser: the other callers must wait */
    runs++;
    init_done = 1;
}
static void *caller(void *arg) {
    (void)arg;
    pthread_once(&once, init);
    if (!init_done) finished_before_return_bad++;
    return NULL;
}

int test_main(void) {
    pthread_t t[8];
    for (int i = 0; i < 8; i++) pthread_create(&t[i], NULL, caller, NULL);
    for (int i = 0; i < 8; i++) pthread_join(t[i], NULL);
    check("the routine ran exactly once for 8 racing threads", runs == 1, runs);
    check("no caller returned before it finished", finished_before_return_bad == 0, finished_before_return_bad);
    int r = pthread_once(&once, init);
    check("a later call returns 0 without running it", r == 0 && runs == 1, runs);

    static pthread_once_t other = PTHREAD_ONCE_INIT;
    r = pthread_once(&other, init);
    check("a second control runs the routine again (2)", r == 0 && runs == 2, runs);
    value("sizeof(pthread_once_t)", sizeof(pthread_once_t));
    return finish();
}
