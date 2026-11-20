/* poly-mac: corefoundation */
/* A dictionary maps keys to values by CFEqual and hash; the count is the
 * number of distinct keys; of equal keys given twice the first pair is kept;
 * the lists are copied. */
#include "poly_mcf.h"

int test_main(void) {
    const void *k[2] = {CFSTR("one"), CFSTR("two")}, *v[2] = {CFSTR("1"), CFSTR("2")};
    CFDictionaryRef d = CFDictionaryCreate(kCFAllocatorDefault, k, v, 2, &kCFTypeDictionaryKeyCallBacks,
                                           &kCFTypeDictionaryValueCallBacks);
    CK(CFDictionaryGetCount(d) == 2);
    CK(CFDictionaryGetValue(d, S("one")) == v[0]);
    CK(CFDictionaryGetValue(d, CFSTR("two")) == v[1]);
    k[0] = CFSTR("changed");
    CK(CFDictionaryContainsKey(d, CFSTR("one")));
    CK(!CFDictionaryContainsKey(d, CFSTR("three")));
    CK(CFDictionaryContainsValue(d, CFSTR("2")));
    CK(!CFDictionaryContainsValue(d, CFSTR("one")));
    CFDictionaryRef e = CFDictionaryCreate(kCFAllocatorDefault, NULL, NULL, 0, &kCFTypeDictionaryKeyCallBacks,
                                           &kCFTypeDictionaryValueCallBacks);
    CK(e != NULL && CFDictionaryGetCount(e) == 0);
    CFDictionaryRef dup = CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){CFSTR("a"), S("a")},
                                             (const void *[]){CFSTR("first"), CFSTR("second")}, 2,
                                             &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CK(CFDictionaryGetCount(dup) == 1);
    CK(cf_is(CFDictionaryGetValue(dup, CFSTR("a")), "first"));
    CFDictionaryRef copy = CFDictionaryCreateCopy(kCFAllocatorDefault, d);
    CK(CFEqual(copy, d));
    return finish();
}
