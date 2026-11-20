/* poly-mac: corefoundation */
/* Every CFRetain adds exactly one; the count of an array, a dictionary, a
 * set, a mutable string and a mutable data object follows the same steps. */
#include "poly_mcf.h"

static void steps(const char *what, CFTypeRef o) {
    CFIndex base = CFGetRetainCount(o);
    CFRetain(o);
    CFRetain(o);
    CFRetain(o);
    check(what, CFGetRetainCount(o) == base + 3, CFGetRetainCount(o) - base);
    CFRelease(o);
    CFRelease(o);
    CFRelease(o);
    check(what, CFGetRetainCount(o) == base, CFGetRetainCount(o) - base);
}

int test_main(void) {
    const void *one[1] = {CFSTR("element")};
    CFArrayRef a = CFArrayCreate(kCFAllocatorDefault, one, 1, &kCFTypeArrayCallBacks);
    steps("array count steps", a);
    CFDictionaryRef d = CFDictionaryCreate(kCFAllocatorDefault, one, one, 1, &kCFTypeDictionaryKeyCallBacks,
                                           &kCFTypeDictionaryValueCallBacks);
    steps("dictionary count steps", d);
    CFSetRef st = CFSetCreate(kCFAllocatorDefault, one, 1, &kCFTypeSetCallBacks);
    steps("set count steps", st);
    CFMutableStringRef s = CFStringCreateMutable(kCFAllocatorDefault, 0);
    steps("mutable string count steps", s);
    CFMutableDataRef m = CFDataCreateMutable(kCFAllocatorDefault, 0);
    steps("mutable data count steps", m);
    return finish();
}
