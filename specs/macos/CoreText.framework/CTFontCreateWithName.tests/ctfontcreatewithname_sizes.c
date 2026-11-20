/* poly-mac: coretext */
/* A size of 0 or a negative size gives the default size 12; very small and very large
 * sizes are kept (0.5, 1000, 4096); the metrics scale with the size exactly (the
 * ascent is 0.8 times it); the size is stored as a float (a size of 12.345 reads
 * back as 12.345 within 0.001). */
#include "poly_mct.h"

int test_main(void) {
    CK(pt_register());
    CKF(CTFontGetSize(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 0, NULL)), 12);
    CKF(CTFontGetSize(CTFontCreateWithName(CFSTR("PolyTest-Regular"), -5, NULL)), 12);
    CKF(CTFontGetSize(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 0.5, NULL)), 0.5);
    CKF(CTFontGetSize(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 1000, NULL)), 1000);
    CKF(CTFontGetSize(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 4096, NULL)), 4096);
    CKF(CTFontGetAscent(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 1000, NULL)), 800);
    CKF(CTFontGetAscent(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 0, NULL)), 9.6);
    CTFontRef odd = CTFontCreateWithName(CFSTR("PolyTest-Regular"), 12.345, NULL);
    CK(CTFontGetSize(odd) > 12.344 && CTFontGetSize(odd) < 12.346);
    return finish();
}
