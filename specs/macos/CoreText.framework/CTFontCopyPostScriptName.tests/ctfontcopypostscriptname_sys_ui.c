/* poly-mac: coretext */
/* SYSTEM-DEPENDENT. The system UI font's PostScript name is '.SFNS-Regular', its full and
 * display name 'System Font Regular', style 'Regular', designer 'Apple Inc.'; the bold
 * UI font '.SFNS-Bold'; the fixed pitch one 'Menlo-Regular'. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef s = CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, 13, NULL);
    CK(cf_is(CTFontCopyPostScriptName(s), ".SFNS-Regular"));
    CK(cf_is(CTFontCopyFullName(s), "System Font Regular") && cf_is(CTFontCopyDisplayName(s), "System Font Regular"));
    CK(cf_is(CTFontCopyName(s, kCTFontStyleNameKey), "Regular") && cf_is(CTFontCopyName(s, kCTFontDesignerNameKey), "Apple Inc."));
    CK(pt_is(CTFontCreateUIFontForLanguage(kCTFontUIFontEmphasizedSystem, 13, NULL), ".SFNS-Bold"));
    CK(pt_is(CTFontCreateUIFontForLanguage(kCTFontUIFontUserFixedPitch, 13, NULL), "Menlo-Regular"));
    return finish();
}
