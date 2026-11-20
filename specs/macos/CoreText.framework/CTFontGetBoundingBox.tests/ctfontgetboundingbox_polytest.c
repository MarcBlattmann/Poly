/* poly-mac: coretext */
/* The font's bounding box is the head table's xMin, yMin, xMax, yMax scaled: (0,-200)-(1400,800)
 * units gives at size 100 the rectangle (0,-20,140,100); at size 10 it is a tenth; the
 * bold font has the same box. */
#include "poly_mct.h"

int test_main(void) {
    CGRect b = CTFontGetBoundingBox(pt_font(0, 100));
    CKF(b.origin.x, 0);
    CKF(b.origin.y, -20);
    CKF(b.size.width, 140);
    CKF(b.size.height, 100);
    CGRect s = CTFontGetBoundingBox(pt_font(0, 10));
    CKF(s.origin.y, -2);
    CKF(s.size.width, 14);
    CKF(s.size.height, 10);
    CGRect bold = CTFontGetBoundingBox(pt_font(1, 100));
    CKF(bold.size.width, 140);
    return finish();
}
