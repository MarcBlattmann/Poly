/* poly-mac: coretext */
/* A TrueType font's bytes make a font descriptor without registering anything: the
 * descriptor names the font (PostScript name, family, style, display name), a font made
 * from it at a size has the metrics of the data, the Bold data gives the Bold font, and
 * the font is not found by name afterwards (the descriptor is not registered). */
#include "poly_mct.h"

static int attr_is(CTFontDescriptorRef d, CFStringRef key, const char *want) {
    CFTypeRef v = CTFontDescriptorCopyAttribute(d, key);
    return v && cf_is(v, want);
}

int test_main(void) {
    CTFontDescriptorRef r = CTFontManagerCreateFontDescriptorFromData(pt_data(0));
    CK(r != NULL && CFGetTypeID(r) == CTFontDescriptorGetTypeID());
    CK(attr_is(r, kCTFontNameAttribute, "PolyTest-Regular"));
    CK(attr_is(r, kCTFontFamilyNameAttribute, "PolyTest"));
    CK(attr_is(r, kCTFontStyleNameAttribute, "Regular"));
    CK(attr_is(r, kCTFontDisplayNameAttribute, "PolyTest Regular"));
    CTFontDescriptorRef b = CTFontManagerCreateFontDescriptorFromData(pt_data(1));
    CK(b != NULL && attr_is(b, kCTFontNameAttribute, "PolyTest-Bold") && attr_is(b, kCTFontStyleNameAttribute, "Bold"));
    CTFontRef f = CTFontCreateWithFontDescriptor(r, 20, NULL);
    CK(pt_is(f, "PolyTest-Regular"));
    CKF(CTFontGetSize(f), 20);
    CKF(CTFontGetAscent(f), 16);
    CKF(CTFontGetDescent(f), 4);
    CK(CTFontGetUnitsPerEm(f) == 1000);
    CK(!pt_is(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL), "PolyTest-Regular"));
    return finish();
}
