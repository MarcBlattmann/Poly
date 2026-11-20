/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* DejaVu Sans applies its standard ligature: 'fi' is one glyph for two characters
 * (range (0,2)), 'ff' too, while 'fa' is two; kerning makes 'AV' narrower than the
 * sum of the advances; the string indices of a ligature glyph are those of its first
 * character. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_file_font("DejaVuSans.ttf", 100);
    CTLineRef fi = pt_line("fi", f);
    CK(CTLineGetGlyphCount(fi) == 1);
    CFRange r = CTLineGetStringRange(fi);
    CK(r.location == 0 && r.length == 2);
    CFIndex idx[1];
    CTRunGetStringIndices(CFArrayGetValueAtIndex(CTLineGetGlyphRuns(fi), 0), CFRangeMake(0, 0), idx);
    CK(idx[0] == 0);
    CK(CTLineGetGlyphCount(pt_line("ff", f)) == 1);
    CK(CTLineGetGlyphCount(pt_line("fa", f)) == 2);
    CK(CTLineGetGlyphCount(pt_line("AV", f)) == 2);
    double av = CTLineGetTypographicBounds(pt_line("AV", f), NULL, NULL, NULL);
    CK(av < 68.41 + 68.41 - 1);
    CKF(av, 130.42);
    return finish();
}
