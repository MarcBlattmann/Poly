/* poly-mac: libsystem
 * Strings are visible through xpc_dictionary_apply as string objects, and xpc_copy_description of a dictionary is a string that
 * starts with "<dictionary:" and mentions the count. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(d, "greeting", "hello");
    xpc_dictionary_set_string(d, "target", "world");
    __block int strings = 0, matches = 0;
    xpc_dictionary_apply(d, ^bool(const char *k, xpc_object_t v) {
        if (xpc_get_type(v) == XPC_TYPE_STRING) strings++;
        if (strcmp(k, "greeting") == 0 && strcmp(xpc_string_get_string_ptr(v), "hello") == 0) matches++;
        if (strcmp(k, "target") == 0 && strcmp(xpc_string_get_string_ptr(v), "world") == 0) matches++;
        return true;
    });
    check("two string values", strings == 2, strings);
    check("each under its key", matches == 2, matches);
    char *desc = xpc_copy_description(d);
    check("the description starts with <dictionary:", strncmp(desc, "<dictionary:", 12) == 0, 0);
    check("and says count = 2", strstr(desc, "count = 2") != NULL, 0);
    free(desc);
    xpc_release(d);
    return finish();
}
