/* poly-mac: coretext */
/* SYSTEM-DEPENDENT. The system font role kCTFontUIFontSystem gives '.SFNS-Regular' (family
 * '.AppleSystemUIFont', full name 'System Font Regular'); a size of 0 gives the role's
 * default size 13, another size is kept (17); the font is a CTFont with the usual
 * metrics (the ascent is positive and the descent too). */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, 0, NULL);
    CK(f != NULL && CFGetTypeID(f) == CTFontGetTypeID());
    CK(pt_is(f, ".SFNS-Regular"));
    CKF(CTFontGetSize(f), 13);
    CTFontRef g = CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, 17, NULL);
    CKF(CTFontGetSize(g), 17);
    CK(cf_is(CTFontCopyFamilyName(g), ".AppleSystemUIFont") && cf_is(CTFontCopyFullName(g), "System Font Regular"));
    CK(CTFontGetAscent(g) > 0 && CTFontGetDescent(g) > 0 && CTFontGetUnitsPerEm(g) > 0);
    CK(CTFontGetSymbolicTraits(g) == 0);
    CK(CFEqual(g, CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, 17, NULL)));
    return finish();
}
