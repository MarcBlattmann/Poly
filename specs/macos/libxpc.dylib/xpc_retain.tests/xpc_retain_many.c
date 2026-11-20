/* poly-mac: libsystem
 * Retained many times, released as many: the object stays usable until the last release (checked up to 1000 retains), and a copy
 * of a dictionary (xpc_copy) is another object with the same contents. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_int64(d, "k", 9);
    int ok = 1;
    for (int i = 0; i < 1000; i++) ok &= xpc_retain(d) == d;
    check("1000 retains all return the object", ok, 0);
    for (int i = 0; i < 1000; i++) xpc_release(d);
    check("after 1000 releases it is still usable", xpc_dictionary_get_int64(d, "k") == 9, 0);
    xpc_object_t c = xpc_copy(d);
    check("xpc_copy is another object with the same contents", c != d && xpc_dictionary_get_int64(c, "k") == 9 && xpc_equal(c, d), 0);
    xpc_dictionary_set_int64(c, "k", 10);
    check("changing the copy leaves the original", xpc_dictionary_get_int64(d, "k") == 9, 0);
    xpc_release(c);
    xpc_release(d);
    return finish();
}
