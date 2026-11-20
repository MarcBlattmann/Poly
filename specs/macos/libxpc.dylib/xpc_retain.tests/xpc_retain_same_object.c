/* poly-mac: libsystem
 * xpc_retain returns the object it was given (the same pointer); a retained object survives one release by the creator and is usable
 * until the second release; the same holds for strings, int64s and dictionaries. */
#include "poly_mapi.h"
int test_main(void) {
    xpc_object_t d = xpc_dictionary_create(NULL, NULL, 0);
    xpc_object_t r = xpc_retain(d);
    check("a dictionary: the same pointer", r == d, 0);
    xpc_release(r);
    xpc_dictionary_set_int64(d, "k", 3);
    check("still usable after one release", xpc_dictionary_get_int64(d, "k") == 3, 0);
    xpc_object_t s = xpc_string_create("abc"), rs = xpc_retain(s);
    check("a string: the same pointer", rs == s, 0);
    xpc_release(s);
    check("the text survives the first release", strcmp(xpc_string_get_string_ptr(rs), "abc") == 0, 0);
    xpc_release(rs);
    xpc_object_t i = xpc_int64_create(77), ri = xpc_retain(i);
    xpc_release(i);
    check("an int64 survives the first release", xpc_int64_get_value(ri) == 77, 0);
    xpc_release(ri);
    xpc_release(d);
    return finish();
}
