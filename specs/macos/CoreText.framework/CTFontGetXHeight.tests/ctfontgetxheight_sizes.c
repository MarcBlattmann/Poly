/* poly-mac: coretext */
/* CTFontGetXHeight is 0.5 times the size for the PolyTest font: OS/2 sxHeight 500; at sizes 10, 12, 16, 100,
 * 1000 and with the default size 0 (12). The value is not rounded to whole units. */
#include "poly_mct.h"

int test_main(void) {
    CKF(CTFontGetXHeight(pt_font(0, 10)), 5.0000);
    CKF(CTFontGetXHeight(pt_font(0, 12)), 6.0000);
    CKF(CTFontGetXHeight(pt_font(0, 16)), 8.0000);
    CKF(CTFontGetXHeight(pt_font(0, 100)), 50.0000);
    CKF(CTFontGetXHeight(pt_font(0, 1000)), 500.0000);
    CKF(CTFontGetXHeight(pt_font(0, 0)), 6.0000);
    return finish();
}
