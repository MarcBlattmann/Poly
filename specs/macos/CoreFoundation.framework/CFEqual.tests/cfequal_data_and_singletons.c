/* poly-mac: corefoundation */
/* Data is equal byte for byte; kCFNull and the two Booleans are single
 * objects equal to themselves only; an object is equal to itself. */
#include "poly_mcf.h"

int test_main(void) {
    UInt8 a[3] = {1, 2, 3}, b[3] = {1, 2, 4};
    CFDataRef d1 = CFDataCreate(kCFAllocatorDefault, a, 3);
    CFDataRef d2 = CFDataCreate(kCFAllocatorDefault, a, 3);
    CFDataRef d3 = CFDataCreate(kCFAllocatorDefault, b, 3);
    CFDataRef d4 = CFDataCreate(kCFAllocatorDefault, a, 2);
    CK(CFEqual(d1, d2));
    CK(!CFEqual(d1, d3));
    CK(!CFEqual(d1, d4));
    CK(CFEqual(CFDataCreate(kCFAllocatorDefault, a, 0), CFDataCreate(kCFAllocatorDefault, b, 0)));
    CK(CFEqual(kCFNull, kCFNull));
    CK(CFEqual(kCFBooleanTrue, kCFBooleanTrue));
    CK(!CFEqual(kCFBooleanTrue, kCFBooleanFalse));
    CK(CFEqual(d1, d1));
    CFMutableDataRef m = CFDataCreateMutable(kCFAllocatorDefault, 0);
    CFDataAppendBytes(m, a, 3);
    CK(CFEqual(m, d1));
    return finish();
}
