/* poly-mac: coretext */
/* A family name alone selects the regular face; adding the bold symbolic trait in
 * kCTFontTraitsAttribute selects the bold one; an unknown name matches nothing (the
 * matching call gives NULL) and the font made from such a descriptor is another font,
 * not PolyTest; an empty dictionary gives a usable descriptor. */
#include "poly_mct.h"

int test_main(void) {
    CK(pt_register());
    CFMutableDictionaryRef fa = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFDictionarySetValue(fa, kCTFontFamilyNameAttribute, CFSTR("PolyTest"));
    CK(pt_is(CTFontCreateWithFontDescriptor(CTFontDescriptorCreateWithAttributes(fa), 20, NULL), "PolyTest-Regular"));
    CFMutableDictionaryRef tr = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    int bold = kCTFontBoldTrait;
    CFDictionarySetValue(tr, kCTFontSymbolicTrait, CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &bold));
    CFDictionarySetValue(fa, kCTFontTraitsAttribute, tr);
    CK(pt_is(CTFontCreateWithFontDescriptor(CTFontDescriptorCreateWithAttributes(fa), 20, NULL), "PolyTest-Bold"));
    CFMutableDictionaryRef nn = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFDictionarySetValue(nn, kCTFontNameAttribute, CFSTR("NoSuchFontZzzQq"));
    CTFontDescriptorRef nd = CTFontDescriptorCreateWithAttributes(nn);
    CK(nd != NULL);
    CK(CTFontDescriptorCreateMatchingFontDescriptor(nd, NULL) == NULL);
    CTFontRef other = CTFontCreateWithFontDescriptor(nd, 20, NULL);
    CK(other != NULL && !pt_is(other, "PolyTest-Regular") && !pt_is(other, "PolyTest-Bold"));
    CFDictionaryRef empty = CFDictionaryCreate(kCFAllocatorDefault, NULL, NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CK(CTFontDescriptorCreateWithAttributes(empty) != NULL);
    return finish();
}
