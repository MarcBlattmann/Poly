/* poly-mac: corefoundation */
/* CFRunLoopGetMain is one object for the life of the process, equal to the
 * main thread's current run loop, with the default mode already known to
 * it. */
#include "poly_mcf.h"

int test_main(void) {
    CFRunLoopRef m = CFRunLoopGetMain();
    CK(m != NULL && CFRunLoopGetMain() == m);
    CK(m == CFRunLoopGetCurrent());
    CFArrayRef modes = CFRunLoopCopyAllModes(m);
    CK(modes != NULL && CFArrayGetCount(modes) >= 1);
    CK(CFArrayContainsValue(modes, CFRangeMake(0, CFArrayGetCount(modes)), kCFRunLoopDefaultMode));
    CFRetain(m);
    CFRelease(m);
    CK(CFRunLoopGetMain() == m && CFRunLoopGetCurrent() == m);
    return finish();
}
