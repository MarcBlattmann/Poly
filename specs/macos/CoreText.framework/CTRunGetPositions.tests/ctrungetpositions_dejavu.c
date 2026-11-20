/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* DejaVu Sans 'AVA' at size 100: the glyph ids are 36, 57, 36; the first position is 0, the
 * second is the kerned advance of A (62.01 = 68.41 less the kern of 6.40) and the third adds the
 * advance of V; the line's width is the last position plus the last advance. */
#include "poly_mct.h"

int test_main(void) {
    CTRunRef r = CFArrayGetValueAtIndex(CTLineGetGlyphRuns(pt_line("AVA", pt_file_font("DejaVuSans.ttf", 100))), 0);
    CGPoint p[3];
    CGSize a[3];
    CGGlyph g[3];
    CTRunGetPositions(r, CFRangeMake(0, 0), p);
    CTRunGetAdvances(r, CFRangeMake(0, 0), a);
    CTRunGetGlyphs(r, CFRangeMake(0, 0), g);
    CK(g[0] == 36 && g[1] == 57 && g[2] == 36);
    CKF(p[0].x, 0);
    CKF(p[1].x, a[0].width);
    CKF(p[2].x, a[0].width + a[1].width);
    CK(a[0].width < 68.41 - 1 && near(a[1].width, a[1].width));
    CKF(p[2].x + a[2].width, CTRunGetTypographicBounds(r, CFRangeMake(0, 0), NULL, NULL, NULL));
    CKF(p[0].y, 0);
    return finish();
}
