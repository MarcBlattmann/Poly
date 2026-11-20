/* poly-mac: coretext */
/* The union is returned also with a NULL rectangles array; a count of 0 gives the zero
 * rectangle; the rectangles scale with the size (size 10: A is (1,0,4,7)); the rectangles
 * are in the glyph's own origin (not moved by its advance position); the bold A is 6
 * units wider at size 100. */
#include "poly_mct.h"

int test_main(void) {
    CGGlyph ag[2] = {2, 7};
    CGRect u = CTFontGetBoundingRectsForGlyphs(pt_font(0, 100), kCTFontOrientationHorizontal, ag, NULL, 2);
    CKF(u.origin.x, 5);
    CKF(u.origin.y, -20);
    CKF(u.size.width, 45);
    CKF(u.size.height, 90);
    CGRect z = CTFontGetBoundingRectsForGlyphs(pt_font(0, 100), kCTFontOrientationHorizontal, ag, NULL, 0);
    CKF(z.size.width, 0);
    CKF(z.size.height, 0);
    CGRect r10;
    CTFontGetBoundingRectsForGlyphs(pt_font(0, 10), kCTFontOrientationHorizontal, ag, &r10, 1);
    CKF(r10.origin.x, 1);
    CKF(r10.size.width, 4);
    CKF(r10.size.height, 7);
    CGRect rb;
    CTFontGetBoundingRectsForGlyphs(pt_font(1, 100), kCTFontOrientationHorizontal, ag, &rb, 1);
    CKF(rb.size.width, 46);
    CGRect rd;
    CTFontGetBoundingRectsForGlyphs(pt_font(0, 100), kCTFontOrientationDefault, ag, &rd, 1);
    CKF(rd.size.width, 40);
    return finish();
}
