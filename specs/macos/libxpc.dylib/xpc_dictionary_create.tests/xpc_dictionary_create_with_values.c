/* poly-mac: libsystem
 * xpc_dictionary_create(keys, values, count) makes a dictionary with the count pairs; the values are retained (releasing them
 * afterwards is fine); the keys are copied; each value keeps its type. */
#include "poly_mapi.h"
int test_main(void) {
    char k1[8] = "alpha", k2[8] = "beta", k3[8] = "gamma";
    const char *keys[3] = {k1, k2, k3};
    xpc_object_t vals[3] = {xpc_int64_create(-7), xpc_string_create("text"), xpc_bool_create(true)};
    xpc_object_t d = xpc_dictionary_create(keys, vals, 3);
    check("count 3", xpc_dictionary_get_count(d) == 3, xpc_dictionary_get_count(d));
    for (int i = 0; i < 3; i++) xpc_release(vals[i]);
    k1[0] = 'z';
    check("the int64 is there", xpc_dictionary_get_int64(d, "alpha") == -7, xpc_dictionary_get_int64(d, "alpha"));
    check("the string is there", strcmp(xpc_dictionary_get_string(d, "beta"), "text") == 0, 0);
    check("the bool is there", xpc_dictionary_get_bool(d, "gamma") == true, 0);
    check("keys were copied: the changed buffer is no key", xpc_dictionary_get_value(d, "zlpha") == NULL, 0);
    check("types are kept", xpc_get_type(xpc_dictionary_get_value(d, "alpha")) == XPC_TYPE_INT64 && xpc_get_type(xpc_dictionary_get_value(d, "beta")) == XPC_TYPE_STRING && xpc_get_type(xpc_dictionary_get_value(d, "gamma")) == XPC_TYPE_BOOL, 0);
    xpc_object_t one = xpc_dictionary_create(keys + 1, vals + 1, 0);
    check("count 0 with non-NULL arrays: empty", xpc_dictionary_get_count(one) == 0, 0);
    xpc_release(d);
    xpc_release(one);
    return finish();
}
