/* exit(2), BSD 1. exit(42): the process ends with status 42. Output written before the call is kept;
 * the recorded exit status is the observation. */
#include "poly_mtest.h"

int test_main(void) {
    check("output before exit is written", 1, 0);
    finish();
    poly_exit(42);
}
