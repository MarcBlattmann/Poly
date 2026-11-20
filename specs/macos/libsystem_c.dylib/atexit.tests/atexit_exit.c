/* atexit: handlers also run for exit(n) and the status is kept; a handler
 * may register another handler, and that one runs next (it is the last
 * registered), before the remaining older ones. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdlib.h>

static void late(void) { put("late handler, registered during exit\n"); }
static void registered_first(void) {
    put("handler registered first: runs last of the two, registers another\n");
    atexit(late);
}
static void registered_second(void) { put("handler registered second: runs first\n"); }

int test_main(void) {
    check("atexit 1", atexit(registered_first) == 0, 0);
    check("atexit 2", atexit(registered_second) == 0, 0);
    put("calling exit(5)\n");
    int st = finish();
    if (st == 0) exit(5);
    exit(6);
}
