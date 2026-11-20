/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf */
/* DejaVu Sans 2.37: CTFontGetCapHeight at size 12 is 8.82 and at size 100 73.54 (its tables scaled, unrounded:
 * cap in font units over 2048 times the size); the bold face has its own value or
 * the same; the metric scales linearly with the size. */
#include "poly_mct.h"

int test_main(void) {
    CKF(CTFontGetCapHeight(pt_file_font("DejaVuSans.ttf", 12)), 8.8200);
    CKF(CTFontGetCapHeight(pt_file_font("DejaVuSans.ttf", 100)), 73.5400);
    CKF(CTFontGetCapHeight(pt_file_font("DejaVuSans.ttf", 200)), 147.0800);
    CK(CTFontGetCapHeight(pt_file_font("DejaVuSans-Bold.ttf", 100)) >= 0);
    return finish();
}
