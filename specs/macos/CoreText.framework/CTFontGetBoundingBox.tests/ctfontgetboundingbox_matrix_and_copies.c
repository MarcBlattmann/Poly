/* poly-mac: coretext */
/* The box follows the size of a copy (CTFontCreateCopyWithAttributes at size 50: width 70),
 * a matrix scaling x by 2 doubles its width and keeps its height; it contains the
 * bounds of every glyph of the font (union of the glyph rectangles). */
#include "poly_mct.h"

int test_main(void) {
    CGRect c = CTFontGetBoundingBox(CTFontCreateCopyWithAttributes(pt_font(0, 10), 50, NULL, NULL));
    CKF(c.size.width, 70);
    CKF(c.size.height, 50);
    CGAffineTransform t = CGAffineTransformMakeScale(2, 1);
    CGRect m = CTFontGetBoundingBox(CTFontCreateWithFontDescriptor(pt_descriptor(0), 100, &t));
    CKF(m.size.width, 280);
    CKF(m.size.height, 100);
    CTFontRef f = pt_font(0, 100);
    CGGlyph g[12];
    for (int i = 0; i < 12; i++) g[i] = (CGGlyph)i;
    CGRect u = CTFontGetBoundingRectsForGlyphs(f, kCTFontOrientationHorizontal, g, NULL, 12);
    CGRect b = CTFontGetBoundingBox(f);
    CK(b.origin.x <= u.origin.x + 0.01 && b.origin.y <= u.origin.y + 0.01);
    CK(b.origin.x + b.size.width >= u.origin.x + u.size.width - 0.01);
    CK(b.origin.y + b.size.height >= u.origin.y + u.size.height - 0.01);
    return finish();
}
