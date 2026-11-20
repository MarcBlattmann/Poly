/* poly-mac: coretext */
/* A descriptor that names a registered font gives that font; a descriptor taken from a font
 * (CTFontCopyFontDescriptor) makes an equal font at the same size and another size;
 * a descriptor with only a family gives the regular face; the font keeps the
 * descriptor's name attribute. */
#include "poly_mct.h"

int test_main(void) {
    CK(pt_register());
    CTFontRef f = CTFontCreateWithName(CFSTR("PolyTest-Bold"), 18, NULL);
    CTFontDescriptorRef d = CTFontCopyFontDescriptor(f);
    CK(d != NULL);
    CK(cf_is(CTFontDescriptorCopyAttribute(d, kCTFontNameAttribute), "PolyTest-Bold"));
    CTFontRef g = CTFontCreateWithFontDescriptor(d, 18, NULL);
    CK(CFEqual(f, g));
    CKF(CTFontGetSize(CTFontCreateWithFontDescriptor(d, 30, NULL)), 30);
    CK(pt_is(CTFontCreateWithFontDescriptor(d, 30, NULL), "PolyTest-Bold"));
    CFMutableDictionaryRef fa = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFDictionarySetValue(fa, kCTFontFamilyNameAttribute, CFSTR("PolyTest"));
    CK(pt_is(CTFontCreateWithFontDescriptor(CTFontDescriptorCreateWithAttributes(fa), 18, NULL), "PolyTest-Regular"));
    CK(CTFontCopyAttribute(f, kCTFontNameAttribute) != NULL);
    return finish();
}
