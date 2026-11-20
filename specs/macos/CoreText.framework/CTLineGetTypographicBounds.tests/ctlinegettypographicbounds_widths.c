/* poly-mac: coretext */
/* The width is the sum of the advances with the font's kern pairs applied: 'AB' 110, 'A' 60,
 * 'AV' 50 + 70 = 120 (A,V -10), 'VA' 62 + 60 = 122 (V,A -8), 'AVA' 172, a space counts 30;
 * the empty line is 0; the return value is a double and ascent, descent and leading are the
 * font's (80, 20, 10) for a non-empty line and 0 for the empty one. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_font(0, 100);
    CGFloat a, d, l;
    CKF(CTLineGetTypographicBounds(pt_line("AB", f), &a, &d, &l), 110);
    CKF(a, 80);
    CKF(d, 20);
    CKF(l, 10);
    CKF(CTLineGetTypographicBounds(pt_line("A", f), NULL, NULL, NULL), 60);
    CKF(CTLineGetTypographicBounds(pt_line("AV", f), NULL, NULL, NULL), 120);
    CKF(CTLineGetTypographicBounds(pt_line("VA", f), NULL, NULL, NULL), 122);
    CKF(CTLineGetTypographicBounds(pt_line("AVA", f), NULL, NULL, NULL), 172);
    CKF(CTLineGetTypographicBounds(pt_line("A A", f), NULL, NULL, NULL), 150);
    CKF(CTLineGetTypographicBounds(pt_line("BB", f), NULL, NULL, NULL), 100);
    CGFloat ea = 9, ed = 9, el = 9;
    CKF(CTLineGetTypographicBounds(pt_line("", f), &ea, &ed, &el), 0);
    CKF(ea, 0);
    CKF(ed, 0);
    CKF(el, 0);
    return finish();
}
