/* poly-mac: libsystem
 * xpc_dictionary_get_int64 gives 0 for a key that is missing, and for a key whose value is not an int64 (a string, a bool, a
 * dictionary, a double); xpc_dictionary_get_value tells missing (NULL) from a stored 0. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(d, "s", "12");
    xpc_dictionary_set_bool(d, "b", true);
    xpc_dictionary_set_double(d, "d", 2.5);
    xpc_dictionary_set_uint64(d, "u", 9);
    xpc_dictionary_set_int64(d, "zero", 0);
    xpc_object_t inner = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_value(d, "dict", inner);
    check("missing: 0", xpc_dictionary_get_int64(d, "nothing") == 0, 0);
    check("a string: 0", xpc_dictionary_get_int64(d, "s") == 0, 0);
    check("a bool: 0", xpc_dictionary_get_int64(d, "b") == 0, 0);
    check("a double: 0", xpc_dictionary_get_int64(d, "d") == 0, 0);
    check("a uint64: 0", xpc_dictionary_get_int64(d, "u") == 0, 0);
    check("a dictionary: 0", xpc_dictionary_get_int64(d, "dict") == 0, 0);
    check("a stored zero is a value, a missing key is not", xpc_dictionary_get_value(d, "zero") != NULL && xpc_dictionary_get_value(d, "nothing") == NULL, 0);
    check("the empty dictionary: 0", xpc_dictionary_get_int64(inner, "k") == 0, 0);
    xpc_release(inner);
    xpc_release(d);
    return finish();
}
