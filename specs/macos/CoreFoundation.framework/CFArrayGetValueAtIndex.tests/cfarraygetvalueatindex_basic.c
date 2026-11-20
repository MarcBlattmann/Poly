/* poly-mac: corefoundation */
/* The value at an index is the very object stored there, for the first and
 * last index and in between, in an array made at once or built by appends. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef s0 = CFSTR("zero"), s1 = CFSTR("one"), s2 = CFSTR("two");
    const void *v[3] = {s0, s1, s2};
    CFArrayRef a = CFArrayCreate(kCFAllocatorDefault, v, 3, &kCFTypeArrayCallBacks);
    CK(CFArrayGetValueAtIndex(a, 0) == s0);
    CK(CFArrayGetValueAtIndex(a, 1) == s1);
    CK(CFArrayGetValueAtIndex(a, 2) == s2);
    CFMutableArrayRef m = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    for (int i = 0; i < 3; i++) CFArrayAppendValue(m, v[i]);
    CK(CFArrayGetValueAtIndex(m, 2) == s2);
    CFNumberRef n = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &(int){5});
    CFArrayAppendValue(m, n);
    CK(CFArrayGetValueAtIndex(m, 3) == n);
    CK(CFGetTypeID(CFArrayGetValueAtIndex(m, 3)) == CFNumberGetTypeID());
    return finish();
}
