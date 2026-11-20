/* poly-mac: corefoundation */
/* A constant string describes itself with its class, address, allocator and
 * contents (addresses masked as 0xADDR). */
#include "poly_mcf.h"

static void d(const char *name, CFTypeRef o) {
    CFStringRef s = CFCopyDescription(o);
    value_masked(name, s);
    if (s) CFRelease(s);
}

int test_main(void) {
    d("constant", CFSTR("hello"));
    d("empty constant", CFSTR(""));
    d("constant with accents", CFSTR("héllo"));
    d("long dynamic string", S("a string that is longer than any short string could be"));
    return finish();
}
