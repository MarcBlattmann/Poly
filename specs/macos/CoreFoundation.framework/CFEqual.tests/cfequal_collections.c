/* poly-mac: corefoundation */
/* Arrays are equal element by element in order, dictionaries and sets without
 * regard to order; elements are compared with CFEqual. */
#include "poly_mcf.h"

int test_main(void) {
    const void *ab[2] = {CFSTR("a"), CFSTR("b")};
    const void *ba[2] = {CFSTR("b"), CFSTR("a")};
    const void *abd[2] = {S("a"), S("b")};
    CFArrayRef a1 = CFArrayCreate(kCFAllocatorDefault, ab, 2, &kCFTypeArrayCallBacks);
    CFArrayRef a2 = CFArrayCreate(kCFAllocatorDefault, abd, 2, &kCFTypeArrayCallBacks);
    CFArrayRef a3 = CFArrayCreate(kCFAllocatorDefault, ba, 2, &kCFTypeArrayCallBacks);
    CFArrayRef a4 = CFArrayCreate(kCFAllocatorDefault, ab, 1, &kCFTypeArrayCallBacks);
    CK(CFEqual(a1, a2));
    CK(!CFEqual(a1, a3));
    CK(!CFEqual(a1, a4));
    CFMutableArrayRef m = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    CFArrayAppendValue(m, CFSTR("a"));
    CFArrayAppendValue(m, CFSTR("b"));
    CK(CFEqual(m, a1));
    const void *k[2] = {CFSTR("k1"), CFSTR("k2")}, *v1[2] = {CFSTR("x"), CFSTR("y")}, *v2[2] = {CFSTR("y"), CFSTR("x")};
    const void *kr[2] = {CFSTR("k2"), CFSTR("k1")};
    CFDictionaryRef d1 = CFDictionaryCreate(kCFAllocatorDefault, k, v1, 2, &kCFTypeDictionaryKeyCallBacks,
                                            &kCFTypeDictionaryValueCallBacks);
    CFDictionaryRef d2 = CFDictionaryCreate(kCFAllocatorDefault, kr, v2, 2, &kCFTypeDictionaryKeyCallBacks,
                                            &kCFTypeDictionaryValueCallBacks);
    CFDictionaryRef d3 = CFDictionaryCreate(kCFAllocatorDefault, k, v2, 2, &kCFTypeDictionaryKeyCallBacks,
                                            &kCFTypeDictionaryValueCallBacks);
    CK(CFEqual(d1, d2));
    CK(!CFEqual(d1, d3));
    CFSetRef s1 = CFSetCreate(kCFAllocatorDefault, ab, 2, &kCFTypeSetCallBacks);
    CFSetRef s2 = CFSetCreate(kCFAllocatorDefault, ba, 2, &kCFTypeSetCallBacks);
    CFSetRef s3 = CFSetCreate(kCFAllocatorDefault, ab, 1, &kCFTypeSetCallBacks);
    CK(CFEqual(s1, s2));
    CK(!CFEqual(s1, s3));
    return finish();
}
