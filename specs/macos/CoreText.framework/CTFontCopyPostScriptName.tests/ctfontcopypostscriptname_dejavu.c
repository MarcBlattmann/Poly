/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSans-Oblique.ttf DejaVuSansMono.ttf */
/* DejaVu: PostScript names DejaVuSans, DejaVuSans-Bold, DejaVuSans-Oblique, DejaVuSansMono;
 * full names 'DejaVu Sans', 'DejaVu Sans Bold', 'DejaVu Sans Oblique', 'DejaVu Sans Mono';
 * style names 'Book', 'Bold', 'Oblique', 'Book'; the version is 'Version 2.37'. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef s = pt_file_font("DejaVuSans.ttf", 12), b = pt_file_font("DejaVuSans-Bold.ttf", 12), o = pt_file_font("DejaVuSans-Oblique.ttf", 12), m = pt_file_font("DejaVuSansMono.ttf", 12);
    CK(pt_is(s, "DejaVuSans") && pt_is(b, "DejaVuSans-Bold") && pt_is(o, "DejaVuSans-Oblique") && pt_is(m, "DejaVuSansMono"));
    CK(cf_is(CTFontCopyFullName(s), "DejaVu Sans") && cf_is(CTFontCopyFullName(b), "DejaVu Sans Bold"));
    CK(cf_is(CTFontCopyFullName(o), "DejaVu Sans Oblique") && cf_is(CTFontCopyFullName(m), "DejaVu Sans Mono"));
    CK(cf_is(CTFontCopyName(s, kCTFontStyleNameKey), "Book") && cf_is(CTFontCopyName(b, kCTFontStyleNameKey), "Bold"));
    CK(cf_is(CTFontCopyName(o, kCTFontStyleNameKey), "Oblique") && cf_is(CTFontCopyName(m, kCTFontStyleNameKey), "Book"));
    CK(cf_is(CTFontCopyName(s, kCTFontVersionNameKey), "Version 2.37"));
    return finish();
}
