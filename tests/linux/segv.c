/* A store to an unmapped address kills the process with SIGSEGV; the diff
 * runner compares the terminating signal. */
#include "poly_test.h"

int test_main(void) {
    /* Through a volatile pointer, so the compiler cannot see the address. */
    volatile int *volatile target = (volatile int *)16;
    put("before the fault\n");
    *target = 1;
    put("not reached\n");
    return 0;
}
