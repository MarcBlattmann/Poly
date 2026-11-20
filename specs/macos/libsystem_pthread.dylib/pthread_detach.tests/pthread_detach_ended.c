/* pthread_detach of a thread that already ended but was not joined: it
 * works (the thread's resources are released); a joined thread's id is
 * ESRCH. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <unistd.h>

static void *quick(void *arg) { return arg; }

int test_main(void) {
    pthread_t t;
    pthread_create(&t, NULL, quick, NULL);
    usleep(100000); /* it has ended, not joined */
    int r = pthread_detach(t);
    check("detach of an ended, unjoined thread returns 0", r == 0, r);
    usleep(50000);
    r = pthread_join(t, NULL);
    check("then it is gone: join gives ESRCH", r == ESRCH, r);

    pthread_create(&t, NULL, quick, NULL);
    pthread_join(t, NULL);
    r = pthread_detach(t);
    check("detach of a joined thread: ESRCH (3)", r == ESRCH, r);

    pthread_t zero = 0;
    r = pthread_detach(zero);
    check("detach of a zero id: ESRCH", r == ESRCH, r);
    return finish();
}
