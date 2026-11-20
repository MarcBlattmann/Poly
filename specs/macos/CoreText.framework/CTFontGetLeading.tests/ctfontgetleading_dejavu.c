/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf */
/* DejaVu Sans 2.37: CTFontGetLeading at size 12 is 0 and at size 100 0 (its tables scaled, unrounded:
 * leading in font units over 2048 times the size); the bold face has its own value or
 * the same; the metric scales linearly with the size. */
#include "poly_mct.h"

int test_main(void) {
    CKF(CTFontGetLeading(pt_file_font("DejaVuSans.ttf", 12)), 0.0000);
    CKF(CTFontGetLeading(pt_file_font("DejaVuSans.ttf", 100)), 0.0000);
    CKF(CTFontGetLeading(pt_file_font("DejaVuSans.ttf", 200)), 0.0000);
    CK(CTFontGetLeading(pt_file_font("DejaVuSans-Bold.ttf", 100)) >= 0);
    return finish();
}
