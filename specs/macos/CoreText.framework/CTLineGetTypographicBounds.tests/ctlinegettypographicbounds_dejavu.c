/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* DejaVu Sans 2.37 at size 100: 'AV' is 130.42 (GPOS kerning), 'Hello, World' 603.61,
 * ascent 92.82, descent 23.58, leading 0; at size 12 the same text is 12/100 of that; the
 * width is not rounded to pixels. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_file_font("DejaVuSans.ttf", 100);
    CGFloat a, d, l;
    CKF(CTLineGetTypographicBounds(pt_line("Hello, World", f), &a, &d, &l), 603.61);
    CKF(a, 92.82);
    CKF(d, 23.58);
    CKF(l, 0);
    CKF(CTLineGetTypographicBounds(pt_line("AV", f), NULL, NULL, NULL), 130.42);
    CKF(CTLineGetTypographicBounds(pt_line("Hello, World", pt_file_font("DejaVuSans.ttf", 12)), NULL, NULL, NULL), 603.61 * 0.12);
    return finish();
}
