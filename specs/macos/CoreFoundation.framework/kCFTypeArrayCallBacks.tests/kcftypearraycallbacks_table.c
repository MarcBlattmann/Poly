/* poly-mac: corefoundation */
/* The table is a CFArrayCallBacks: version 0, all four functions present, retain
 * and release working on CF objects, equal being CFEqual, and copyDescription
 * giving a string. */
#include "poly_mcf.h"

int test_main(void) {
    const CFArrayCallBacks *cb = &kCFTypeArrayCallBacks;
    CK(cb->version == 0);
    CK(cb->retain && cb->release && cb->copyDescription && cb->equal);
    CK(cb->equal == (CFArrayEqualCallBack)CFEqual);
    CFDataRef d = CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"x", 1);
    const void *r = cb->retain(kCFAllocatorDefault, d);
    CK(r == d && CFGetRetainCount(d) == 2);
    cb->release(kCFAllocatorDefault, d);
    CK(CFGetRetainCount(d) == 1);
    CK(cb->equal(CFSTR("a"), S("a")) && !cb->equal(CFSTR("a"), S("b")));
    CFStringRef desc = cb->copyDescription(CFSTR("text"));
    CK(desc != NULL && CFGetTypeID(desc) == CFStringGetTypeID());
    CK(sizeof(CFArrayCallBacks) == 40);
    return finish();
}
