/* poly-mac: coretext */
/* SYSTEM-DEPENDENT. The system font created by name '.AppleSystemUIFont' is the same font
 * as the system role font at that size (same PostScript name, size and metrics; the two objects are not CFEqual); the font descriptor's name attribute reads
 * '.SFNS-Regular'; a language argument 'en' or 'de' does not change the Latin font; the
 * font has glyphs for Latin letters and an advance for 'A' that is positive. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef role = CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, 13, NULL);
    CTFontRef byname = CTFontCreateWithName(CFSTR(".AppleSystemUIFont"), 13, NULL);
    CK(pt_is(role, ".SFNS-Regular") && pt_is(byname, ".SFNS-Regular"));
    CKF(CTFontGetSize(byname), 13);
    CKF(CTFontGetAscent(role), CTFontGetAscent(byname));
    CK(cf_is(CTFontDescriptorCopyAttribute(CTFontCopyFontDescriptor(role), kCTFontNameAttribute), ".SFNS-Regular"));
    CK(pt_is(CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, 13, CFSTR("en")), ".SFNS-Regular"));
    CK(pt_is(CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, 13, CFSTR("de")), ".SFNS-Regular"));
    UniChar a = 'A';
    CGGlyph g = 0;
    CK(CTFontGetGlyphsForCharacters(role, &a, &g, 1) && g != 0);
    CGSize adv;
    CK(CTFontGetAdvancesForGlyphs(role, kCTFontOrientationHorizontal, &g, &adv, 1) > 0);
    return finish();
}
