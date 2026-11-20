/* poly-mac: corefoundation */
/* With the standard callbacks the array retains an appended object and
 * releases it again when it is removed or the array goes away. */
#include "poly_mcf.h"

int test_main(void) {
    CFDataRef d = CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"x", 1);
    CFMutableArrayRef m = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    CFArrayAppendValue(m, d);
    CKV(CFGetRetainCount(d) == 2, CFGetRetainCount(d));
    CFArrayAppendValue(m, d);
    CKV(CFGetRetainCount(d) == 3, CFGetRetainCount(d));
    CFArrayRemoveValueAtIndex(m, 0);
    CKV(CFGetRetainCount(d) == 2, CFGetRetainCount(d));
    CFArrayRemoveAllValues(m);
    CKV(CFGetRetainCount(d) == 1, CFGetRetainCount(d));
    CFArrayAppendValue(m, d);
    CFArraySetValueAtIndex(m, 0, kCFNull);
    CKV(CFGetRetainCount(d) == 1, CFGetRetainCount(d));
    CFRelease(m);
    CKV(CFGetRetainCount(d) == 1, CFGetRetainCount(d));
    return finish();
}
