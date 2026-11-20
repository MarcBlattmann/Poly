/* poly-mac: libsystem
 * The pointer xpc_dictionary_get_string returns belongs to the stored string: it reads the same bytes for as long as the string is
 * stored, also after the dictionary was retained and released once more; removing the key by storing NULL leaves the dictionary
 * without it. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    xpc_dictionary_set_string(d, "name", "poly");
    const char *p = xpc_dictionary_get_string(d, "name");
    xpc_retain(d);
    xpc_release(d);
    check("after a retain and a release the text is the same", strcmp(p, "poly") == 0 && xpc_dictionary_get_string(d, "name") == p, 0);
    xpc_dictionary_set_int64(d, "x", 1);
    xpc_dictionary_set_int64(d, "y", 2);
    check("after more keys the text is the same", strcmp(xpc_dictionary_get_string(d, "name"), "poly") == 0, 0);
    xpc_dictionary_set_value(d, "name", NULL);
    check("storing NULL removes it", xpc_dictionary_get_string(d, "name") == NULL && xpc_dictionary_get_count(d) == 2, 0);
    xpc_release(d);
    return finish();
}
