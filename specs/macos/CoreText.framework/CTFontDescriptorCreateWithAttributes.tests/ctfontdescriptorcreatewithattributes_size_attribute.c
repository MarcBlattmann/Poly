/* poly-mac: coretext */
/* A kCTFontSizeAttribute in the descriptor is the size of a font made from it when the
 * size argument is 0, and the argument wins when it is not 0; with no size attribute a
 * size of 0 gives 12; the attribute reads back as a number. */
#include "poly_mct.h"

int test_main(void) {
    CK(pt_register());
    CFMutableDictionaryRef pa = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFDictionarySetValue(pa, kCTFontNameAttribute, CFSTR("PolyTest-Regular"));
    CTFontDescriptorRef plain = CTFontDescriptorCreateWithAttributes(pa);
    CFMutableDictionaryRef at = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFDictionarySetValue(at, kCTFontNameAttribute, CFSTR("PolyTest-Regular"));
    CKF(CTFontGetSize(CTFontCreateWithFontDescriptor(plain, 0, NULL)), 12);
    double sz = 25;
    CFDictionarySetValue(at, kCTFontSizeAttribute, CFNumberCreate(kCFAllocatorDefault, kCFNumberDoubleType, &sz));
    CTFontDescriptorRef sized = CTFontDescriptorCreateWithAttributes(at);
    CKF(CTFontGetSize(CTFontCreateWithFontDescriptor(sized, 0, NULL)), 25);
    CKF(CTFontGetSize(CTFontCreateWithFontDescriptor(sized, 30, NULL)), 30);
    double back = 0;
    CFTypeRef v = CTFontDescriptorCopyAttribute(sized, kCTFontSizeAttribute);
    CK(v != NULL && CFGetTypeID(v) == CFNumberGetTypeID() && CFNumberGetValue(v, kCFNumberDoubleType, &back) && back == 25);
    CK(CTFontDescriptorCopyAttribute(plain, kCTFontSizeAttribute) == NULL);
    return finish();
}
