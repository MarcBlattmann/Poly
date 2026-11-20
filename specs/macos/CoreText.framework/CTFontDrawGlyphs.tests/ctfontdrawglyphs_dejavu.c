/* poly-mac: coretext
 * poly-mac: fonts DejaVuSans.ttf */
/* DejaVu glyphs drawn with CTFontDrawGlyphs land where CTLineDraw puts them: 'AV' drawn as a line
 * at (4,10) and the same glyphs drawn at the run's positions give the same image, pixel for
 * pixel; the ink is within the glyphs' bounding rectangles. */
#include "poly_mct.h"

int test_main(void) {
    CTFontRef f = pt_file_font("DejaVuSans.ttf", 24);
    CTLineRef l = pt_line("AV", f);
    unsigned char *px;
    CGContextRef c = pt_bitmap(70, 40, 255, &px);
    CGContextSetShouldSmoothFonts(c, 0);
    CGContextSetGrayFillColor(c, 0.0, 1.0);
    CGContextSetTextPosition(c, 4, 10);
    CTLineDraw(l, c);
    static unsigned char first[70 * 40];
    for (int i = 0; i < 70 * 40; i++) first[i] = px[i];
    CTRunRef r = CFArrayGetValueAtIndex(CTLineGetGlyphRuns(l), 0);
    CGGlyph g[2];
    CGPoint pos[2];
    CTRunGetGlyphs(r, CFRangeMake(0, 0), g);
    CTRunGetPositions(r, CFRangeMake(0, 0), pos);
    pos[0].x += 4; pos[0].y += 10; pos[1].x += 4; pos[1].y += 10;
    unsigned char *q;
    CGContextRef c2 = pt_bitmap(70, 40, 255, &q);
    CGContextSetShouldSmoothFonts(c2, 0);
    CGContextSetGrayFillColor(c2, 0.0, 1.0);
    CTFontDrawGlyphs(f, g, pos, 2, c2);
    int same = 1, ink = 0;
    for (int i = 0; i < 70 * 40; i++) {
        same &= first[i] == q[i];
        ink += q[i] < 0xf0;
    }
    CK(same);
    CK(ink > 40);
    return finish();
}
