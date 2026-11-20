/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSansMono.ttf */
/* DejaVu Sans 2.37 has 6253 glyphs, DejaVu Sans Bold 6196 and DejaVu Sans Mono 3377 (the
 * maxp numGlyphs of the files), the same at every size. */
#include "poly_mct.h"

int test_main(void) {
    CK(CTFontGetGlyphCount(pt_file_font("DejaVuSans.ttf", 12)) == 6253);
    CK(CTFontGetGlyphCount(pt_file_font("DejaVuSans.ttf", 400)) == 6253);
    CK(CTFontGetGlyphCount(pt_file_font("DejaVuSans-Bold.ttf", 12)) == CTFontGetGlyphCount(pt_file_font("DejaVuSans-Bold.ttf", 99)));
    CK(CTFontGetGlyphCount(pt_file_font("DejaVuSans-Bold.ttf", 12)) == 6196);
    CK(CTFontGetGlyphCount(pt_file_font("DejaVuSansMono.ttf", 12)) == 3377);
    return finish();
}
