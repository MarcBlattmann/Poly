/* Writes to stdout and stderr: the diff runner keeps the two streams apart. */
#include "poly_test.h"

int test_main(void) {
    put_fd(1, "to stdout\n");
    put_fd(2, "to stderr\n");
    put_fd(1, "stdout again\n");
    return 0;
}
