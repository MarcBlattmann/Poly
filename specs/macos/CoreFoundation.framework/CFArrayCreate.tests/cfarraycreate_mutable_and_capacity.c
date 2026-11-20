/* poly-mac: corefoundation */
/* A mutable array starts empty whatever capacity it was given (a hint, not a
 * limit) and grows past it; CFArrayCreateMutableCopy copies the values and
 * leaves the original alone. */
#include "poly_mcf.h"

int test_main(void) {
    CFMutableArrayRef m = CFArrayCreateMutable(kCFAllocatorDefault, 2, &kCFTypeArrayCallBacks);
    CK(CFArrayGetCount(m) == 0);
    for (int i = 0; i < 100; i++) {
        CFArrayAppendValue(m, CFStringCreateWithFormat(kCFAllocatorDefault, NULL, CFSTR("%d"), i));
    }
    CKV(CFArrayGetCount(m) == 100, CFArrayGetCount(m));
    CK(cf_is(CFArrayGetValueAtIndex(m, 99), "99"));
    CFMutableArrayRef c = CFArrayCreateMutableCopy(kCFAllocatorDefault, 0, m);
    CFArrayRemoveAllValues(c);
    CK(CFArrayGetCount(c) == 0 && CFArrayGetCount(m) == 100);
    CFMutableArrayRef d = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    CK(CFArrayGetCount(d) == 0);
    return finish();
}
