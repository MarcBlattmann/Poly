/* poly-mac: corefoundation */
/* The standard key callbacks at work: keys that are equal objects but not the
 * same object find each other; keys of different classes are different keys
 * (the string "1" and the number 1); a key is retained by the dictionary. */
#include "poly_mcf.h"

int test_main(void) {
    CFMutableDictionaryRef d = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    int one = 1;
    CFNumberRef n1 = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &one);
    CFDictionarySetValue(d, CFSTR("1"), CFSTR("string one"));
    CFDictionarySetValue(d, n1, CFSTR("number one"));
    CK(CFDictionaryGetCount(d) == 2);
    CK(cf_is(CFDictionaryGetValue(d, S("1")), "string one"));
    long long one64 = 1;
    CK(cf_is(CFDictionaryGetValue(d, CFNumberCreate(kCFAllocatorDefault, kCFNumberLongLongType, &one64)), "number one"));
    CFDataRef k = CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"key", 3);
    CFDictionarySetValue(d, k, CFSTR("data"));
    CKV(CFGetRetainCount(k) == 2, CFGetRetainCount(k));
    CK(cf_is(CFDictionaryGetValue(d, CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"key", 3)), "data"));
    CFDictionaryRemoveValue(d, k);
    CKV(CFGetRetainCount(k) == 1, CFGetRetainCount(k));
    return finish();
}
