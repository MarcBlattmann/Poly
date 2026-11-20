/* poly-mac: coretext */
/* The glyph count is the maxp table's numGlyphs: 12 for both PolyTest fonts, at any size, for
 * a copy and with a matrix; glyph ids 0..11 have outlines or advances, 12 does not. */
#include "poly_mct.h"

int test_main(void) {
    CK(CTFontGetGlyphCount(pt_font(0, 10)) == 12);
    CK(CTFontGetGlyphCount(pt_font(0, 1000)) == 12);
    CK(CTFontGetGlyphCount(pt_font(1, 20)) == 12);
    CK(CTFontGetGlyphCount(CTFontCreateCopyWithAttributes(pt_font(0, 10), 50, NULL, NULL)) == 12);
    CGAffineTransform t = CGAffineTransformMakeScale(2, 2);
    CK(CTFontGetGlyphCount(CTFontCreateWithFontDescriptor(pt_descriptor(0), 20, &t)) == 12);
    CTFontRef f = pt_font(0, 100);
    CGGlyph last[1] = {11}, past[1] = {12};
    CGSize a;
    CK(CTFontCreatePathForGlyph(f, last[0], NULL) != NULL);
    CK(CTFontCreatePathForGlyph(f, past[0], NULL) == NULL);
    CKF(CTFontGetAdvancesForGlyphs(f, kCTFontOrientationHorizontal, last, &a, 1), 80);
    return finish();
}
