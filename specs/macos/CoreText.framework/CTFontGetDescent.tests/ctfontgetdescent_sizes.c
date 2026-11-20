/* poly-mac: coretext */
/* CTFontGetDescent is 0.2 times the size for the PolyTest font: a positive number, from hhea descender -200 (typo -300 and win 250 are not used); at sizes 10, 12, 16, 100,
 * 1000 and with the default size 0 (12). The value is not rounded to whole units. */
#include "poly_mct.h"

int test_main(void) {
    CKF(CTFontGetDescent(pt_font(0, 10)), 2.0000);
    CKF(CTFontGetDescent(pt_font(0, 12)), 2.4000);
    CKF(CTFontGetDescent(pt_font(0, 16)), 3.2000);
    CKF(CTFontGetDescent(pt_font(0, 100)), 20.0000);
    CKF(CTFontGetDescent(pt_font(0, 1000)), 200.0000);
    CKF(CTFontGetDescent(pt_font(0, 0)), 2.4000);
    return finish();
}
