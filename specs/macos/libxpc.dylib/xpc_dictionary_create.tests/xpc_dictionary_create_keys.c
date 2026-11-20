/* poly-mac: libsystem
 * Keys of a dictionary: the empty string is a key; keys are case-sensitive; a 299-character key works; setting the same key
 * again replaces the value (count unchanged); different keys that share a prefix are different. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_int64(d, "", 3);
    check("the empty key works", xpc_dictionary_get_count(d) == 1 && xpc_dictionary_get_int64(d, "") == 3, 0);
    xpc_dictionary_set_int64(d, "Key", 1);
    xpc_dictionary_set_int64(d, "key", 2);
    check("keys are case-sensitive", xpc_dictionary_get_int64(d, "Key") == 1 && xpc_dictionary_get_int64(d, "key") == 2, 0);
    xpc_dictionary_set_int64(d, "key2", 5);
    check("a longer key with the same prefix is another key", xpc_dictionary_get_int64(d, "key") == 2 && xpc_dictionary_get_int64(d, "key2") == 5, 0);
    char big[300];
    memset(big, 'k', 299);
    big[299] = 0;
    xpc_dictionary_set_int64(d, big, 9);
    check("a 299-character key works", xpc_dictionary_get_int64(d, big) == 9, 0);
    big[298] = 'j';
    check("and differs from one that differs in its last character", xpc_dictionary_get_value(d, big) == NULL, 0);
    size_t n = xpc_dictionary_get_count(d);
    xpc_dictionary_set_int64(d, "key", 20);
    check("setting a key again replaces the value", xpc_dictionary_get_int64(d, "key") == 20 && xpc_dictionary_get_count(d) == n, 0);
    xpc_dictionary_set_value(d, "key", NULL);
    check("a NULL value removes the key", xpc_dictionary_get_count(d) == n - 1 && xpc_dictionary_get_value(d, "key") == NULL, 0);
    xpc_release(d);
    return finish();
}
