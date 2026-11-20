/* poly-mac: libsystem
 * Many keys: 200 int64 values go in under different keys and all read back; the count is 200; xpc_dictionary_apply visits each
 * of them once (in no promised order) and stops when the block says false. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    char key[16];
    for (int i = 0; i < 200; i++) {
        snprintf(key, sizeof key, "key%d", i);
        xpc_dictionary_set_int64(d, key, i * 3);
    }
    check("count 200", xpc_dictionary_get_count(d) == 200, xpc_dictionary_get_count(d));
    int ok = 1;
    for (int i = 0; i < 200; i++) {
        snprintf(key, sizeof key, "key%d", i);
        ok &= xpc_dictionary_get_int64(d, key) == i * 3;
    }
    check("all read back", ok, 0);
    __block int visited = 0;
    __block long sum = 0;
    bool completed = xpc_dictionary_apply(d, ^bool(const char *k, xpc_object_t v) {
        (void)k;
        visited++;
        sum += xpc_int64_get_value(v);
        return true;
    });
    check("apply visits each once and reports completion", completed && visited == 200, visited);
    check("with the right values", sum == 3L * 199 * 200 / 2, sum);
    __block int stopped = 0;
    completed = xpc_dictionary_apply(d, ^bool(const char *k, xpc_object_t v) {
        (void)k;
        (void)v;
        stopped++;
        return stopped < 5;
    });
    check("returning false stops it after that one and reports it", !completed && stopped == 5, stopped);
    xpc_release(d);
    return finish();
}
