/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSansMono.ttf */
/* The value is a count of units, an integer: DejaVu Sans, DejaVu Sans Bold and DejaVu
 * Sans Mono have 2048, at every size; a font made from the file at size 12 and at 500 agree. */
#include "poly_mct.h"

int test_main(void) {
    CK(CTFontGetUnitsPerEm(pt_file_font("DejaVuSans.ttf", 12)) == 2048);
    CK(CTFontGetUnitsPerEm(pt_file_font("DejaVuSans.ttf", 500)) == 2048);
    CK(CTFontGetUnitsPerEm(pt_file_font("DejaVuSans-Bold.ttf", 12)) == 2048);
    CK(CTFontGetUnitsPerEm(pt_file_font("DejaVuSansMono.ttf", 12)) == 2048);
    CTFontRef big = pt_file_font("DejaVuSans.ttf", 100);
    CK(CTFontGetUnitsPerEm(big) == 2048 && CTFontGetUnitsPerEm(CTFontCreateCopyWithAttributes(big, 9, NULL, NULL)) == 2048);
    return finish();
}
