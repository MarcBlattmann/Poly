/* poly-mac: corefoundation */
/* The Boolean constants are ordinary collection members and dictionary values:
 * what comes out is the same constant, and a Boolean key does not clash with
 * a number key. */
#include "poly_mcf.h"

int test_main(void) {
    CFDictionaryRef d = CFDictionaryCreate(kCFAllocatorDefault, (const void *[]){CFSTR("on"), CFSTR("off")},
                                           (const void *[]){kCFBooleanTrue, kCFBooleanFalse}, 2,
                                           &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CK(CFDictionaryGetValue(d, CFSTR("on")) == kCFBooleanTrue);
    CK(CFDictionaryGetValue(d, CFSTR("off")) == kCFBooleanFalse);
    CFArrayRef a = CFArrayCreate(kCFAllocatorDefault, (const void *[]){kCFBooleanTrue, kCFBooleanFalse}, 2, &kCFTypeArrayCallBacks);
    CK(CFArrayGetValueAtIndex(a, 0) == kCFBooleanTrue);
    CK(CFBooleanGetValue(CFArrayGetValueAtIndex(a, 1)) == 0);
    CFSetRef s = CFSetCreate(kCFAllocatorDefault, (const void *[]){kCFBooleanTrue, kCFBooleanTrue, kCFBooleanFalse}, 3, &kCFTypeSetCallBacks);
    CK(CFSetGetCount(s) == 2);
    CK(CFNumberCompare(CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &(int){1}), CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &(int){1}), NULL) == 0);
    return finish();
}
