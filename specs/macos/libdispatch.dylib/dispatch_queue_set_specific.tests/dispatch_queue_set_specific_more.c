/* Specific values are pointers compared by address: two keys with the
 * same content are different keys; the value pointer is returned as given. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    static char k1[4] = "key", k2[4] = "key";
    dispatch_queue_t q = dispatch_queue_create("com.poly.addr", DISPATCH_QUEUE_SERIAL);
    int object = 5;
    dispatch_queue_set_specific(q, k1, &object, NULL);
    check("the key is its address: k1 finds the value", dispatch_queue_get_specific(q, k1) == &object, 0);
    check("another array with the same text is another key", dispatch_queue_get_specific(q, k2) == NULL, 0);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block void *seen = (void *)1;
    dispatch_async(q, ^{ seen = dispatch_get_specific(k1); dispatch_semaphore_signal(done); });
    gcd_wait(done);
    check("a block sees the value pointer as given", seen == &object, 0);
    return finish();
}
