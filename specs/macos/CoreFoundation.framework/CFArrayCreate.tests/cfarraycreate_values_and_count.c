/* poly-mac: corefoundation */
/* An array keeps its values in order, its count is the count given, an empty
 * array is allowed (also with a NULL value list), and the value list is copied. */
#include "poly_mcf.h"

int test_main(void) {
    const void *v[3] = {CFSTR("a"), CFSTR("b"), CFSTR("c")};
    CFArrayRef a = CFArrayCreate(kCFAllocatorDefault, v, 3, &kCFTypeArrayCallBacks);
    CK(CFArrayGetCount(a) == 3);
    CK(CFArrayGetValueAtIndex(a, 0) == v[0] && CFArrayGetValueAtIndex(a, 2) == v[2]);
    v[0] = CFSTR("changed");
    CK(cf_is(CFArrayGetValueAtIndex(a, 0), "a"));
    CFArrayRef two = CFArrayCreate(kCFAllocatorDefault, v, 2, &kCFTypeArrayCallBacks);
    CK(CFArrayGetCount(two) == 2);
    CFArrayRef e = CFArrayCreate(kCFAllocatorDefault, NULL, 0, &kCFTypeArrayCallBacks);
    CK(e != NULL && CFArrayGetCount(e) == 0);
    CFArrayRef dup = CFArrayCreate(kCFAllocatorDefault, (const void *[]){CFSTR("x"), CFSTR("x")}, 2, &kCFTypeArrayCallBacks);
    CK(CFArrayGetCount(dup) == 2);
    CFArrayRef copy = CFArrayCreateCopy(kCFAllocatorDefault, a);
    CK(CFArrayGetCount(copy) == 3 && CFEqual(copy, a));
    return finish();
}
