/* poly-mac: libsystem
 * Values of nested dictionaries: a dictionary stored in another (set_value) is retained, so releasing it afterwards keeps the
 * stored one valid; xpc_dictionary_get_dictionary gives the same object, NULL for a missing key or a value that is not a dictionary;
 * a change made through the inner object shows in the outer one. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t outer = xpc_dictionary_create(NULL, NULL, 0), inner = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_int64(inner, "n", 42);
    xpc_dictionary_set_value(outer, "inner", inner);
    xpc_release(inner);
    xpc_object_t got = xpc_dictionary_get_dictionary(outer, "inner");
    check("after the owner released it, it is still there", got == inner && xpc_dictionary_get_int64(got, "n") == 42, 0);
    xpc_dictionary_set_int64(got, "m", 7);
    check("a change through the inner object shows in the outer one", xpc_dictionary_get_int64(xpc_dictionary_get_dictionary(outer, "inner"), "m") == 7, 0);
    check("a missing key gives NULL", xpc_dictionary_get_dictionary(outer, "nothing") == NULL, 0);
    xpc_dictionary_set_int64(outer, "i", 1);
    check("a value that is not a dictionary gives NULL", xpc_dictionary_get_dictionary(outer, "i") == NULL, 0);
    check("a non-dictionary object gives 0 for get_int64", xpc_dictionary_get_int64(got, "nothing") == 0, 0);
    xpc_release(outer);
    return finish();
}
