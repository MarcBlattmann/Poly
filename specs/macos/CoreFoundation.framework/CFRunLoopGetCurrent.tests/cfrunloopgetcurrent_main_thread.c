/* poly-mac: corefoundation */
/* On the main thread the current run loop is the main run loop, the same object
 * every time, a CFRunLoop; outside a run it has no current mode; the
 * mode names are the constant strings kCFRunLoopDefaultMode and
 * kCFRunLoopCommonModes. */
#include "poly_mcf.h"

int test_main(void) {
    CFRunLoopRef cur = CFRunLoopGetCurrent();
    CK(cur != NULL);
    CK(CFRunLoopGetCurrent() == cur);
    CK(CFRunLoopGetMain() == cur);
    CK(CFGetTypeID(cur) == CFRunLoopGetTypeID());
    CK(CFRunLoopCopyCurrentMode(cur) == NULL);
    CK(cf_is(kCFRunLoopDefaultMode, "kCFRunLoopDefaultMode"));
    CK(cf_is(kCFRunLoopCommonModes, "kCFRunLoopCommonModes"));
    CK(!CFRunLoopIsWaiting(cur));
    return finish();
}
