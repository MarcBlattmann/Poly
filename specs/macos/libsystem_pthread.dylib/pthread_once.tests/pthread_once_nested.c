/* pthread_once: two different controls from within each other's routine;
 * the routine may call other once-controls. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>

static pthread_once_t outer = PTHREAD_ONCE_INIT, inner = PTHREAD_ONCE_INIT;
static int order[4], n;

static void inner_fn(void) { order[n++] = 2; }
static void outer_fn(void) {
    order[n++] = 1;
    pthread_once(&inner, inner_fn);
    order[n++] = 3;
}

int test_main(void) {
    int r = pthread_once(&outer, outer_fn);
    check("outer returns 0", r == 0, r);
    check("order 1 (outer starts), 2 (inner), 3 (outer ends)", order[0] == 1 && order[1] == 2 && order[2] == 3 && n == 3, n);
    pthread_once(&inner, inner_fn);
    pthread_once(&outer, outer_fn);
    check("nothing runs again", n == 3, n);
    return finish();
}
