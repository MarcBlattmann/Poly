/* poly-mac: corefoundation */
/* Objects that CFEqual calls equal have the same hash, whatever way they
 * were made or which type their numbers carry. */
#include "poly_mcf.h"

int test_main(void) {
    CK(CFHash(S("a text to hash")) == CFHash(CFSTR("a text to hash")));
    CFMutableStringRef m = CFStringCreateMutable(kCFAllocatorDefault, 0);
    CFStringAppendCString(m, "a text to hash", kCFStringEncodingUTF8);
    CK(CFHash(m) == CFHash(CFSTR("a text to hash")));
    int i = 42;
    long long l = 42;
    short sh = 42;
    double d = 42.0;
    float f = 42.0f;
    CFNumberRef ni = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &i);
    CK(CFHash(ni) == CFHash(CFNumberCreate(kCFAllocatorDefault, kCFNumberLongLongType, &l)));
    CK(CFHash(ni) == CFHash(CFNumberCreate(kCFAllocatorDefault, kCFNumberShortType, &sh)));
    CK(CFHash(ni) == CFHash(CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &d)));
    CK(CFHash(ni) == CFHash(CFNumberCreate(kCFAllocatorDefault, kCFNumberFloatType, &f)));
    UInt8 b[4] = {9, 8, 7, 6};
    CK(CFHash(CFDataCreate(kCFAllocatorDefault, b, 4)) == CFHash(CFDataCreate(kCFAllocatorDefault, b, 4)));
    const void *v[2] = {CFSTR("x"), CFSTR("y")};
    const void *vd[2] = {S("x"), S("y")};
    CK(CFHash(CFArrayCreate(kCFAllocatorDefault, v, 2, &kCFTypeArrayCallBacks)) ==
       CFHash(CFArrayCreate(kCFAllocatorDefault, vd, 2, &kCFTypeArrayCallBacks)));
    CK(CFHash(CFSetCreate(kCFAllocatorDefault, v, 2, &kCFTypeSetCallBacks)) ==
       CFHash(CFSetCreate(kCFAllocatorDefault, vd, 2, &kCFTypeSetCallBacks)));
    return finish();
}
