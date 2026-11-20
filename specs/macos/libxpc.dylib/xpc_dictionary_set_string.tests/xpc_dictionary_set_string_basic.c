/* poly-mac: libsystem
 * xpc_dictionary_set_string(dict, key, value) stores a copy of the string (changing the source afterwards does not change it); the
 * object has type XPC_TYPE_STRING; xpc_string_get_length and xpc_string_get_string_ptr agree; the empty string is stored; setting
 * again replaces. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    char src[16] = "hello";
    xpc_dictionary_set_string(d, "s", src);
    src[0] = 'J';
    check("stored as a copy", strcmp(xpc_dictionary_get_string(d, "s"), "hello") == 0, 0);
    xpc_object_t v = xpc_dictionary_get_value(d, "s");
    check("a string object", xpc_get_type(v) == XPC_TYPE_STRING && xpc_string_get_length(v) == 5 && strcmp(xpc_string_get_string_ptr(v), "hello") == 0, 0);
    xpc_dictionary_set_string(d, "empty", "");
    check("the empty string is stored", xpc_dictionary_get_value(d, "empty") != NULL && strcmp(xpc_dictionary_get_string(d, "empty"), "") == 0 && xpc_string_get_length(xpc_dictionary_get_value(d, "empty")) == 0, 0);
    xpc_dictionary_set_string(d, "s", "other");
    check("again replaces", strcmp(xpc_dictionary_get_string(d, "s"), "other") == 0 && xpc_dictionary_get_count(d) == 2, 0);
    xpc_dictionary_set_string(d, "utf8", "gr\xc3\xbc\xc3\x9f" "e \xe2\x82\xac");
    check("UTF-8 bytes are kept", strcmp(xpc_dictionary_get_string(d, "utf8"), "gr\xc3\xbc\xc3\x9f" "e \xe2\x82\xac") == 0 && xpc_string_get_length(xpc_dictionary_get_value(d, "utf8")) == 11, xpc_string_get_length(xpc_dictionary_get_value(d, "utf8")));
    char big[1001];
    memset(big, 'x', 1000);
    big[1000] = 0;
    xpc_dictionary_set_string(d, "big", big);
    check("a 1000-character string", xpc_string_get_length(xpc_dictionary_get_value(d, "big")) == 1000, 0);
    xpc_release(d);
    return finish();
}
