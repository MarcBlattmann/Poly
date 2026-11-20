/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSans-Oblique.ttf DejaVuSans-BoldOblique.ttf DejaVuSansMono.ttf */
/* DejaVu: Sans 0, Sans Bold bold, Sans Oblique italic (the post italic angle gives a negative
 * slant angle, -11 degrees), Sans Bold Oblique both, Sans Mono monospace. */
#include "poly_mct.h"

int test_main(void) {
    CK(CTFontGetSymbolicTraits(pt_file_font("DejaVuSans.ttf", 12)) == 0);
    CK(CTFontGetSymbolicTraits(pt_file_font("DejaVuSans-Bold.ttf", 12)) == kCTFontTraitBold);
    CTFontRef o = pt_file_font("DejaVuSans-Oblique.ttf", 12);
    CK(CTFontGetSymbolicTraits(o) == kCTFontTraitItalic);
    CKF(CTFontGetSlantAngle(o), -11);
    CK(CTFontGetSymbolicTraits(pt_file_font("DejaVuSans-BoldOblique.ttf", 12)) == (kCTFontTraitBold | kCTFontTraitItalic));
    CK(CTFontGetSymbolicTraits(pt_file_font("DejaVuSansMono.ttf", 12)) == kCTFontTraitMonoSpace);
    CKF(CTFontGetSlantAngle(pt_file_font("DejaVuSans.ttf", 12)), 0);
    return finish();
}
