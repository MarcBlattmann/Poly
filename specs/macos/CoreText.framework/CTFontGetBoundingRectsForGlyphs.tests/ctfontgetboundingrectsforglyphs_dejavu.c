/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* DejaVu Sans A at size 100: the outline's exact bounds (0.78, 0, 66.80, 72.90), the same
 * for V; a is (6.01, -1.42, 46.19, 57.42) (its curves overshoot the baseline); the space
 * has the zero rectangle. The bounds are not rounded to whole pixels. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_file_font("DejaVuSans.ttf", 100);
    UniChar chs[] = {'A', 'V', 'a', ' '};
    CGGlyph g[4];
    CK(CTFontGetGlyphsForCharacters(f, chs, g, 4));
    CGRect r[4];
    CTFontGetBoundingRectsForGlyphs(f, kCTFontOrientationHorizontal, g, r, 4);
    CKF(r[0].origin.x, 0.78);
    CKF(r[0].origin.y, 0);
    CKF(r[0].size.width, 66.80);
    CKF(r[0].size.height, 72.90);
    CKF(r[1].size.width, 66.80);
    CKF(r[2].origin.x, 6.01);
    CKF(r[2].origin.y, -1.42);
    CKF(r[2].size.width, 46.19);
    CKF(r[2].size.height, 57.42);
    CKF(r[3].size.width, 0);
    CKF(r[3].size.height, 0);
    return finish();
}
