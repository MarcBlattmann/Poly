/* poly-mac: coretext */
/* A registered font is created by PostScript name, full name or family name at the size
 * asked (20 stays 20, 10.5 stays 10.5, a matrix argument of NULL is the identity); the
 * result has the registered font's metrics and names, each call a distinct font
 * object that is equal (CFEqual) to another made with the same name and size. */
#include "poly_mct.h"

int test_main(void) {
    CK(pt_register());
    CTFontRef f = CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL);
    CK(f != NULL && CFGetTypeID(f) == CTFontGetTypeID());
    CK(pt_is(f, "PolyTest-Regular"));
    CKF(CTFontGetSize(f), 20);
    CKF(CTFontGetAscent(f), 16);
    CKF(CTFontGetXHeight(f), 10);
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest Bold"), 20, NULL), "PolyTest-Bold"));
    CK(pt_is(CTFontCreateWithName(CFSTR("PolyTest"), 20, NULL), "PolyTest-Regular"));
    CKF(CTFontGetSize(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 10.5, NULL)), 10.5);
    CKF(CTFontGetAscent(CTFontCreateWithName(CFSTR("PolyTest-Regular"), 10.5, NULL)), 8.4);
    CTFontRef g = CTFontCreateWithName(CFSTR("PolyTest-Regular"), 20, NULL);
    CK(CFEqual(f, g));
    CK(!CFEqual(f, CTFontCreateWithName(CFSTR("PolyTest-Regular"), 21, NULL)));
    CK(!CFEqual(f, CTFontCreateWithName(CFSTR("PolyTest-Bold"), 20, NULL)));
    return finish();
}
