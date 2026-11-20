/* poly-mac: corefoundation */
/* Append adds at the end, one at a time, growing the count by one; the same
 * value may be appended again. */
#include "poly_mcf.h"

int test_main(void) {
    CFMutableArrayRef m = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    CFStringRef a = CFSTR("a"), b = CFSTR("b");
    CFArrayAppendValue(m, a);
    CKV(CFArrayGetCount(m) == 1, CFArrayGetCount(m));
    CFArrayAppendValue(m, b);
    CFArrayAppendValue(m, a);
    CKV(CFArrayGetCount(m) == 3, CFArrayGetCount(m));
    CK(CFArrayGetValueAtIndex(m, 0) == a && CFArrayGetValueAtIndex(m, 1) == b && CFArrayGetValueAtIndex(m, 2) == a);
    CK(CFArrayGetFirstIndexOfValue(m, CFRangeMake(0, 3), b) == 1);
    CK(CFArrayGetFirstIndexOfValue(m, CFRangeMake(1, 2), a) == 2);
    CK(CFArrayGetLastIndexOfValue(m, CFRangeMake(0, 3), a) == 2);
    CK(CFArrayContainsValue(m, CFRangeMake(0, 3), S("b")));
    CK(!CFArrayContainsValue(m, CFRangeMake(0, 3), S("q")));
    CK(CFArrayGetFirstIndexOfValue(m, CFRangeMake(0, 3), S("q")) == kCFNotFound);
    CK(CFArrayGetCountOfValue(m, CFRangeMake(0, 3), a) == 2);
    return finish();
}
