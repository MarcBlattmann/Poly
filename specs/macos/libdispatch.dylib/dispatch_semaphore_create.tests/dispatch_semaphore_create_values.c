/* dispatch_semaphore_create(n): a counting semaphore with n tokens; a
 * negative initial value is refused with NULL. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static int try_wait(dispatch_semaphore_t s) { return dispatch_semaphore_wait(s, DISPATCH_TIME_NOW) == 0; }

int test_main(void) {
    check("a negative initial value gives NULL", dispatch_semaphore_create(-1) == NULL, 0);
    check("so does a large negative one", dispatch_semaphore_create(-1000) == NULL, 0);
    dispatch_semaphore_t z = dispatch_semaphore_create(0);
    check("0 gives a semaphore", z != NULL, 0);
    check("with no tokens: a wait with DISPATCH_TIME_NOW fails (returns nonzero)", !try_wait(z), 0);

    dispatch_semaphore_t three = dispatch_semaphore_create(3);
    int got = 0;
    for (int i = 0; i < 5; i++) got += try_wait(three);
    check("created with 3: exactly three waits succeed", got == 3, got);

    dispatch_semaphore_t big = dispatch_semaphore_create(100000);
    got = 0;
    for (int i = 0; i < 100001; i++) got += try_wait(big);
    check("created with 100000: exactly 100000 waits succeed", got == 100000, got);

    dispatch_semaphore_t a = dispatch_semaphore_create(1), b = dispatch_semaphore_create(1);
    check("two semaphores are different objects", a != b, 0);
    try_wait(a);
    check("taking from one does not touch the other", try_wait(b), 0);
    return finish();
}
