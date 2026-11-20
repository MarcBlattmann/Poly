/* poly-mac: libsystem
 * Releasing the outer dictionary releases what it stored: an object retained before stays valid, one that was only stored is
 * gone with it (not testable without crashing, so only the retained case is checked); releasing a dictionary that was stored in
 * another one and then removed from it leaves the other holder's reference. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t outer = xpc_dictionary_create(NULL, NULL, 0), inner = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_int64(inner, "n", 5);
    xpc_dictionary_set_value(outer, "inner", inner);
    xpc_release(outer);
    check("the inner dictionary, still held by the test, is usable after the outer one was released", xpc_dictionary_get_int64(inner, "n") == 5, 0);
    xpc_object_t a = xpc_dictionary_create(NULL, NULL, 0), b = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_value(a, "x", inner);
    xpc_dictionary_set_value(b, "x", inner);
    xpc_dictionary_set_value(a, "x", NULL);
    check("removed from one holder it stays in the other", xpc_dictionary_get_int64(xpc_dictionary_get_dictionary(b, "x"), "n") == 5, 0);
    xpc_release(a);
    xpc_release(inner);
    check("the second holder keeps it after the test's own reference is gone", xpc_dictionary_get_int64(xpc_dictionary_get_dictionary(b, "x"), "n") == 5, 0);
    xpc_release(b);
    return finish();
}
