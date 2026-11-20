/* poly-mac: corefoundation */
/* A mutable variant has the type identifier of its immutable class. */
#include "poly_mcf.h"

int test_main(void) {
    CK(CFGetTypeID(CFStringCreateMutable(kCFAllocatorDefault, 0)) == CFStringGetTypeID());
    CK(CFGetTypeID(CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks)) == CFArrayGetTypeID());
    CK(CFGetTypeID(CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks,
                                             &kCFTypeDictionaryValueCallBacks)) == CFDictionaryGetTypeID());
    CK(CFGetTypeID(CFSetCreateMutable(kCFAllocatorDefault, 0, &kCFTypeSetCallBacks)) == CFSetGetTypeID());
    CK(CFGetTypeID(CFDataCreateMutable(kCFAllocatorDefault, 0)) == CFDataGetTypeID());
    CFStringRef s = S("copy me please, a long enough text");
    CK(CFGetTypeID(CFStringCreateMutableCopy(kCFAllocatorDefault, 0, s)) == CFStringGetTypeID());
    return finish();
}
