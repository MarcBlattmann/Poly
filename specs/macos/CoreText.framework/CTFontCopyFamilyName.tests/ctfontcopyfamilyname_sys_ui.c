/* poly-mac: coretext */
/* SYSTEM-DEPENDENT. The system UI font's family is '.AppleSystemUIFont'; Helvetica's family
 * is 'Helvetica'; the default font for an unknown name is Helvetica too. */
#include "poly_mct.h"

int test_main(void) {
    CK(cf_is(CTFontCopyFamilyName(CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, 13, NULL)), ".AppleSystemUIFont"));
    CK(cf_is(CTFontCopyFamilyName(CTFontCreateWithName(CFSTR("Helvetica"), 13, NULL)), "Helvetica"));
    CK(cf_is(CTFontCopyFamilyName(CTFontCreateWithName(CFSTR("NoSuchFontZzz"), 13, NULL)), "Helvetica"));
    return finish();
}
