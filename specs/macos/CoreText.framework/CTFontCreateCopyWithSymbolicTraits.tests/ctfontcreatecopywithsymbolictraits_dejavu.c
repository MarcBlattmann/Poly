/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSans-Oblique.ttf DejaVuSans-BoldOblique.ttf */
/* With the DejaVu Sans faces registered, the bold copy of Sans is Sans-Bold, the italic
 * copy Sans-Oblique, both Sans-BoldOblique, and dropping the bold bit of
 * Sans-BoldOblique gives Sans-Oblique; the size is kept. */
#include "poly_mct.h"

int test_main(void) {
    const char *files[] = {"DejaVuSans.ttf", "DejaVuSans-Bold.ttf", "DejaVuSans-Oblique.ttf", "DejaVuSans-BoldOblique.ttf"};
    CK(pt_register_files(files, 4) == 4);
    CTFontRef sans = CTFontCreateWithName(CFSTR("DejaVuSans"), 14, NULL);
    CK(pt_is(sans, "DejaVuSans"));
    CK(pt_is(CTFontCreateCopyWithSymbolicTraits(sans, 0, NULL, kCTFontTraitBold, kCTFontTraitBold), "DejaVuSans-Bold"));
    CK(pt_is(CTFontCreateCopyWithSymbolicTraits(sans, 0, NULL, kCTFontTraitItalic, kCTFontTraitItalic), "DejaVuSans-Oblique"));
    CTFontRef both = CTFontCreateCopyWithSymbolicTraits(sans, 0, NULL, kCTFontTraitBold | kCTFontTraitItalic, kCTFontTraitBold | kCTFontTraitItalic);
    CK(pt_is(both, "DejaVuSans-BoldOblique"));
    CK(pt_is(CTFontCreateCopyWithSymbolicTraits(both, 0, NULL, 0, kCTFontTraitBold), "DejaVuSans-Oblique"));
    CKF(CTFontGetSize(both), 14);
    return finish();
}
