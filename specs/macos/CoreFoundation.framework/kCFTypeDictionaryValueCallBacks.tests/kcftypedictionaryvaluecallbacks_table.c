/* poly-mac: corefoundation */
/* The value table: version 0, retain, release, copyDescription and equal present
 * (no hash); equal is CFEqual; retain and release count; dictionaries made with
 * it compare values with it. */
#include "poly_mcf.h"

int test_main(void) {
    const CFDictionaryValueCallBacks *cb = &kCFTypeDictionaryValueCallBacks;
    CK(cb->version == 0);
    CK(cb->retain && cb->release && cb->copyDescription && cb->equal);
    CK(cb->equal == (CFDictionaryEqualCallBack)CFEqual);
    CFDataRef d = CFDataCreate(kCFAllocatorDefault, (const UInt8 *)"x", 1);
    CK(cb->retain(kCFAllocatorDefault, d) == d && CFGetRetainCount(d) == 2);
    cb->release(kCFAllocatorDefault, d);
    CK(CFGetRetainCount(d) == 1);
    CK(sizeof(CFDictionaryValueCallBacks) == 40);
    CFDictionaryRef a = CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){CFSTR("k")}, (const void *[]){S("v")}, 1,
                                           &kCFTypeDictionaryKeyCallBacks, cb);
    CFDictionaryRef b = CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){CFSTR("k")}, (const void *[]){S("v")}, 1,
                                           &kCFTypeDictionaryKeyCallBacks, cb);
    CK(CFEqual(a, b));
    CK(CFDictionaryContainsValue(a, S("v")));
    return finish();
}
