/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSans-Oblique.ttf DejaVuSansMono.ttf */
/* DejaVu: the family of Sans, Sans Bold and Sans Oblique is 'DejaVu Sans' (the faces of
 * one family share it), that of the monospaced one 'DejaVu Sans Mono'. */
#include "poly_mct.h"

int test_main(void) {
    CK(cf_is(CTFontCopyFamilyName(pt_file_font("DejaVuSans.ttf", 12)), "DejaVu Sans"));
    CK(cf_is(CTFontCopyFamilyName(pt_file_font("DejaVuSans-Bold.ttf", 12)), "DejaVu Sans"));
    CK(cf_is(CTFontCopyFamilyName(pt_file_font("DejaVuSans-Oblique.ttf", 12)), "DejaVu Sans"));
    CK(cf_is(CTFontCopyFamilyName(pt_file_font("DejaVuSansMono.ttf", 12)), "DejaVu Sans Mono"));
    CFStringRef lang = NULL;
    CK(cf_is(CTFontCopyLocalizedName(pt_file_font("DejaVuSans.ttf", 12), kCTFontFamilyNameKey, &lang), "DejaVu Sans") && cf_is(lang, "en"));
    return finish();
}
