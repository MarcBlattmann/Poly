/* poly-mac: coretext */
/* CTFontDrawGlyphs draws glyphs at given positions, each position the glyph's origin: A at (1,2)
 * and B at (7,2) in the 14 x 12 gray bitmap give the same two black boxes as the line 'AB' (A 4x7
 * at columns 2..5, B 3x5 at columns 8..10), and nothing else; a count of 0 draws nothing;
 * glyph 0 (.notdef) is drawn as its box (x 0.5..4.5 from the origin: half covered at both
 * ends). */
#include "poly_mct.h"

int test_main(void) {
    unsigned char *px;
    CGContextRef c = pt_bitmap(14, 12, 255, &px);
    CGContextSetShouldAntialias(c, 1);
    CGContextSetShouldSmoothFonts(c, 0);
    CGContextSetGrayFillColor(c, 0.0, 1.0);
    CGGlyph g[2] = {2, 3};
    CGPoint p[2] = {{1, 2}, {7, 2}};
    CTFontDrawGlyphs(pt_font(0, 10), g, p, 2, c);
    int ok = 1;
    for (int y = 0; y < 12; y++)
        for (int x = 0; x < 14; x++) {
            int pa = x >= 2 && x < 6 && y >= 3 && y < 10, pb = x >= 8 && x < 11 && y >= 5 && y < 10;
            if (px[y * 14 + x] != ((pa || pb) ? 0x00 : 0xff)) ok = 0;
        }
    CK(ok);
    pt_dump("two", px, 14, 12);
    unsigned char *q;
    CGContextRef c2 = pt_bitmap(14, 12, 255, &q);
    CGContextSetShouldSmoothFonts(c2, 0);
    CTFontDrawGlyphs(pt_font(0, 10), g, p, 0, c2);
    int blank = 1;
    for (int i = 0; i < 14 * 12; i++) blank &= q[i] == 0xff;
    CK(blank);
    CGGlyph nd[1] = {0};
    CGPoint np[1] = {{1, 2}};
    CGContextRef c3 = pt_bitmap(14, 12, 255, &q);
    CGContextSetShouldAntialias(c3, 1);
    CGContextSetShouldSmoothFonts(c3, 0);
    CGContextSetGrayFillColor(c3, 0.0, 1.0);
    CTFontDrawGlyphs(pt_font(0, 10), nd, np, 1, c3);
    CK(q[5 * 14 + 2] == 0 && q[5 * 14 + 4] == 0 && q[5 * 14 + 6] == 0xff && q[2 * 14 + 2] == 0xff);
    CK(q[5 * 14 + 1] > 0x70 && q[5 * 14 + 1] < 0x90 && q[5 * 14 + 5] > 0x70 && q[5 * 14 + 5] < 0x90);
    return finish();
}
