/* poly-mac: corefoundation */
/* Constant objects are not counted: a retain of kCFNull, a Boolean, a CFSTR
 * string or the default allocator changes nothing and returns the same object. */
#include "poly_mcf.h"

static void immortal(const char *what, CFTypeRef o) {
    CFIndex before = CFGetRetainCount(o);
    CFTypeRef r = CFRetain(o);
    check(what, r == o && CFGetRetainCount(o) == before, CFGetRetainCount(o) - before);
}

int test_main(void) {
    immortal("kCFNull", kCFNull);
    immortal("kCFBooleanTrue", kCFBooleanTrue);
    immortal("kCFBooleanFalse", kCFBooleanFalse);
    immortal("CFSTR", CFSTR("constant"));
    immortal("empty CFSTR", CFSTR(""));
    CK(CFGetRetainCount(kCFNull) > 1000);
    CK(CFGetRetainCount(CFSTR("constant")) > 1000);
    return finish();
}
