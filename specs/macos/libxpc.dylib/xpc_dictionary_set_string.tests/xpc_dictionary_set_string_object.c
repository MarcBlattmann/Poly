/* poly-mac: libsystem
 * xpc_string_create makes an equal string to the stored one (xpc_equal); a string made with xpc_string_create_with_format reads as
 * formatted; the string pointer of a stored string stays valid and the same while it is stored and unchanged. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(d, "s", "value");
    xpc_object_t made = xpc_string_create("value");
    check("equal to xpc_string_create of the same text", xpc_equal(xpc_dictionary_get_value(d, "s"), made), 0);
    xpc_object_t other = xpc_string_create("Value");
    check("not equal to a different text", !xpc_equal(xpc_dictionary_get_value(d, "s"), other), 0);
    xpc_object_t f = xpc_string_create_with_format("%s-%d", "item", 7);
    check("create_with_format", strcmp(xpc_string_get_string_ptr(f), "item-7") == 0 && xpc_string_get_length(f) == 6, 0);
    const char *p1 = xpc_dictionary_get_string(d, "s"), *p2 = xpc_dictionary_get_string(d, "s");
    check("the pointer is the same while it is stored", p1 == p2, 0);
    xpc_dictionary_set_int64(d, "other", 1);
    check("and after other keys were added", xpc_dictionary_get_string(d, "s") == p1, 0);
    xpc_release(made);
    xpc_release(other);
    xpc_release(f);
    xpc_release(d);
    return finish();
}
