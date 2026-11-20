/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf */
/* A real font's file (DejaVu Sans 2.37, copied into the scratch directory) gives a
 * descriptor with its names ('DejaVuSans', family 'DejaVu Sans', style 'Book'; the
 * Bold file 'DejaVuSans-Bold', style 'Bold'), and a font from it has the real metrics
 * and glyph count. */
#include "poly_mct.h"

static int attr_is(CTFontDescriptorRef d, CFStringRef key, const char *want) {
    CFTypeRef v = CTFontDescriptorCopyAttribute(d, key);
    return v && cf_is(v, want);
}

int test_main(void) {
    CFDataRef reg = pt_read_file("DejaVuSans.ttf"), bold = pt_read_file("DejaVuSans-Bold.ttf");
    CK(reg != NULL && bold != NULL);
    CTFontDescriptorRef d = CTFontManagerCreateFontDescriptorFromData(reg);
    CK(d != NULL);
    CK(attr_is(d, kCTFontNameAttribute, "DejaVuSans"));
    CK(attr_is(d, kCTFontFamilyNameAttribute, "DejaVu Sans"));
    CK(attr_is(d, kCTFontStyleNameAttribute, "Book"));
    CTFontDescriptorRef db = CTFontManagerCreateFontDescriptorFromData(bold);
    CK(db != NULL && attr_is(db, kCTFontNameAttribute, "DejaVuSans-Bold") && attr_is(db, kCTFontStyleNameAttribute, "Bold"));
    CTFontRef f = CTFontCreateWithFontDescriptor(d, 100, NULL);
    CK(CTFontGetUnitsPerEm(f) == 2048);
    CKF(CTFontGetAscent(f), 92.82);
    CKF(CTFontGetDescent(f), 23.58);
    CK(CTFontGetGlyphCount(f) == 6253);
    return finish();
}
