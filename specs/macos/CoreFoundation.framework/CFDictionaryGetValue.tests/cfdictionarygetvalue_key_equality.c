/* poly-mac: corefoundation */
/* Keys are found by CFEqual and CFHash, not by address: a number key finds a
 * number of another type with the same value; a string key is
 * case sensitive; keys of different classes do not clash. */
#include "poly_mcf.h"

int test_main(void) {
    int one = 1;
    long long one64 = 1;
    CFNumberRef kn = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &one);
    CFDictionaryRef d = CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){kn, CFSTR("Key")},
                                           (const void *[]){CFSTR("num"), CFSTR("str")}, 2,
                                           &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CK(cf_is(CFDictionaryGetValue(d, CFNumberCreate(kCFAllocatorDefault, kCFNumberLongLongType, &one64)), "num"));
    CK(cf_is(CFDictionaryGetValue(d, S("Key")), "str"));
    CK(CFDictionaryGetValue(d, S("key")) == NULL);
    CK(CFDictionaryGetValue(d, S("1")) == NULL);
    CK(CFDictionaryGetCount(d) == 2);
    return finish();
}
