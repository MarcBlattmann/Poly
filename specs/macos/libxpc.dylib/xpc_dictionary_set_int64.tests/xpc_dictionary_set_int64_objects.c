/* poly-mac: libsystem
 * What is stored by xpc_dictionary_set_int64 is a real XPC object: xpc_int64_create makes an equal one, xpc_equal agrees, the object
 * can be taken out with get_value, retained, put in another dictionary and the value read there. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0), e = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_int64(d, "n", 123);
    xpc_object_t made = xpc_int64_create(123);
    check("xpc_equal with xpc_int64_create(123)", xpc_equal(xpc_dictionary_get_value(d, "n"), made), 0);
    xpc_object_t other = xpc_int64_create(124);
    check("and not with 124", !xpc_equal(xpc_dictionary_get_value(d, "n"), other), 0);
    xpc_object_t v = xpc_retain(xpc_dictionary_get_value(d, "n"));
    xpc_dictionary_set_value(e, "m", v);
    check("put into another dictionary it reads the same", xpc_dictionary_get_int64(e, "m") == 123, 0);
    xpc_dictionary_set_value(d, "n", NULL);
    check("removed from the first it stays in the second", xpc_dictionary_get_int64(e, "m") == 123 && xpc_dictionary_get_count(d) == 0, 0);
    xpc_release(v);
    xpc_release(made);
    xpc_release(other);
    xpc_release(d);
    xpc_release(e);
    return finish();
}
