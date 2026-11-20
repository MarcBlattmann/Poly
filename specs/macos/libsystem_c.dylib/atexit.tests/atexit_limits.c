/* atexit: many handlers can be registered (more than the 32 the C standard
 * requires) and all run. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <stdlib.h>

static int ran;
static void h(void) { ran++; }
static void report(void) {
    put("handlers run: ");
    put_num(ran);
    put("\n");
}

int test_main(void) {
    atexit(report); /* registered first: runs last */
    int fails = 0;
    for (int i = 0; i < 1000; i++)
        if (atexit(h) != 0) fails++;
    check("1000 registrations all return 0", fails == 0, fails);
    return finish();
}
