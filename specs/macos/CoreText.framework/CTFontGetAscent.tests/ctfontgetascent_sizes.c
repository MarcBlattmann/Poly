/* poly-mac: coretext */
/* CTFontGetAscent is 0.8 times the size for the PolyTest font: hhea ascender (800 units; the OS/2 typo 700 and win 900 values are not used); at sizes 10, 12, 16, 100,
 * 1000 and with the default size 0 (12). The value is not rounded to whole units. */
#include "poly_mct.h"

int test_main(void) {
    CKF(CTFontGetAscent(pt_font(0, 10)), 8.0000);
    CKF(CTFontGetAscent(pt_font(0, 12)), 9.6000);
    CKF(CTFontGetAscent(pt_font(0, 16)), 12.8000);
    CKF(CTFontGetAscent(pt_font(0, 100)), 80.0000);
    CKF(CTFontGetAscent(pt_font(0, 1000)), 800.0000);
    CKF(CTFontGetAscent(pt_font(0, 0)), 9.6000);
    return finish();
}
