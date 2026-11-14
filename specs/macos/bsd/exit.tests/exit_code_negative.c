/* exit(2), BSD 1. exit(-1): status 255. Output written before the call is kept;
 * the recorded exit status is the observation. */
#include "poly_mtest.h"

int test_main(void) {
    check("output before exit is written", 1, 0);
    finish();
    poly_exit(-1);
}
