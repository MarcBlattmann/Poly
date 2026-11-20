/* poly-mac: corefoundation */
/* The standard collection callbacks retain what goes in: an element's count is
 * one higher while the array holds it, and an array retained twice holds its
 * elements once. */
#include "poly_mcf.h"

int test_main(void) {
    CFDataRef d = CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"xy", 2);
    CFIndex base = CFGetRetainCount(d);
    const void *vals[1] = {d};
    CFArrayRef a = CFArrayCreate(kCFAllocatorDefault, vals, 1, &kCFTypeArrayCallBacks);
    CKV(CFGetRetainCount(d) == base + 1, CFGetRetainCount(d) - base);
    CFRetain(a);
    CKV(CFGetRetainCount(d) == base + 1, CFGetRetainCount(d) - base);
    CFRelease(a);
    CFRelease(a);
    CKV(CFGetRetainCount(d) == base, CFGetRetainCount(d) - base);
    CFRelease(d);
    return finish();
}
