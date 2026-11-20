/* poly-mac: corefoundation */
/* The bytes are copied at creation; the length is the length given; the
 * bytes can be read through a pointer or copied out by range. */
#include "poly_mcf.h"

int test_main(void) {
    UInt8 b[4] = {1, 2, 3, 4};
    CFDataRef d = CFDataCreate(kCFAllocatorDefault, b, 4);
    b[0] = 9;
    CK(CFDataGetLength(d) == 4);
    CK(CFDataGetBytePtr(d)[0] == 1 && CFDataGetBytePtr(d)[3] == 4);
    UInt8 out[8];
    memset(out, 0xee, sizeof out);
    CFDataGetBytes(d, CFRangeMake(1, 2), out);
    CK(out[0] == 2 && out[1] == 3 && out[2] == 0xee);
    CFDataRef part = CFDataCreate(kCFAllocatorDefault, b, 2);
    CK(CFDataGetLength(part) == 2);
    CFDataRef copy = CFDataCreateCopy(kCFAllocatorDefault, d);
    CK(CFEqual(copy, d) && CFDataGetLength(copy) == 4);
    CFDataRef sub = CFDataCreateWithBytesNoCopy(kCFAllocatorDefault, b, 4, kCFAllocatorNull);
    CK(CFDataGetBytePtr(sub) == b);
    return finish();
}
