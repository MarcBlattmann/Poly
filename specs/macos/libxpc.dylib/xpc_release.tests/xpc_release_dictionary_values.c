/* poly-mac: libsystem
 * Values made for a dictionary are released by the caller after storing them (the dictionary holds its own reference): ten
 * strings made, stored and released read back; replacing a value releases the old one (the old one made and released by the test
 * is not touched); xpc_dictionary_set_value of a value that is also another key's keeps both. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    char key[8], val[8];
    for (int i = 0; i < 10; i++) {
        snprintf(key, sizeof key, "k%d", i);
        snprintf(val, sizeof val, "v%d", i);
        xpc_object_t s = xpc_string_create(val);
        xpc_dictionary_set_value(d, key, s);
        xpc_release(s);
    }
    int ok = 1;
    for (int i = 0; i < 10; i++) {
        snprintf(key, sizeof key, "k%d", i);
        snprintf(val, sizeof val, "v%d", i);
        ok &= strcmp(xpc_dictionary_get_string(d, key), val) == 0;
    }
    check("ten stored and released strings read back", ok, 0);
    xpc_object_t shared = xpc_string_create("shared");
    xpc_dictionary_set_value(d, "one", shared);
    xpc_dictionary_set_value(d, "two", shared);
    xpc_release(shared);
    xpc_dictionary_set_value(d, "one", NULL);
    check("a value stored under two keys stays under the other", strcmp(xpc_dictionary_get_string(d, "two"), "shared") == 0, 0);
    xpc_object_t repl = xpc_string_create("new");
    xpc_dictionary_set_value(d, "k0", repl);
    xpc_release(repl);
    check("a replaced key shows the new value", strcmp(xpc_dictionary_get_string(d, "k0"), "new") == 0, 0);
    xpc_release(d);
    return finish();
}
