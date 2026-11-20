/* A notify on the main queue waits for the main thread to drain it. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    dispatch_group_t g = dispatch_group_create();
    __block int ran = 0;
    dispatch_group_notify(g, dispatch_get_main_queue(), ^{ ran = 1; });
    gcd_pause_ms(200);
    check("the group was empty but the main queue is not being drained: nothing ran", ran == 0, ran);
    return finish();
}
