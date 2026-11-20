/* poly-mac: coretext */
/* A glyph without outline (space) and a glyph id past the font's count give NULL; .notdef
 * (glyph 0) has its box (5,0,40,70); the transform argument is applied to the points
 * (scale (2,1): A is (20,0)..(100,70)); the size 10 path is a tenth of the size 100 one;
 * a NULL transform is the identity. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_font(0, 100);
    CK(CTFontCreatePathForGlyph(f, 1, NULL) == NULL);
    CK(CTFontCreatePathForGlyph(f, 40, NULL) == NULL);
    CK(CTFontCreatePathForGlyph(f, 12, NULL) == NULL);
    CGRect n = CGPathGetBoundingBox(CTFontCreatePathForGlyph(f, 0, NULL));
    CKF(n.origin.x, 5);
    CKF(n.size.width, 40);
    CKF(n.size.height, 70);
    CGAffineTransform t = CGAffineTransformMakeScale(2, 1);
    CGRect s = CGPathGetBoundingBox(CTFontCreatePathForGlyph(f, 2, &t));
    CKF(s.origin.x, 20);
    CKF(s.size.width, 80);
    CKF(s.size.height, 70);
    CGRect small = CGPathGetBoundingBox(CTFontCreatePathForGlyph(pt_font(0, 10), 2, NULL));
    CKF(small.origin.x, 1);
    CKF(small.size.width, 4);
    CKF(small.size.height, 7);
    CGAffineTransform id = CGAffineTransformIdentity;
    CGRect same = CGPathGetBoundingBox(CTFontCreatePathForGlyph(f, 2, &id));
    CKF(same.size.width, 40);
    return finish();
}
