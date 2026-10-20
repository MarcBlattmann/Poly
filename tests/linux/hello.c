/* Static "hello world": one write to stdout, then exit_group(0). The first
 * program Poly has to run (roadmap Phase 3). */
#include "poly_test.h"

int test_main(void) {
    put("hello, world\n");
    return 0;
}
