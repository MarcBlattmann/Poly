/* poly-mac: coretext */
/* Drawing a line into an 8-bit gray bitmap context (white background, black fill, font smoothing
 * and antialiasing as set below): the box glyphs of PolyTest at size 10 with the text position
 * (1,2) cover whole pixels, so the result is pure black and white: A is 4 pixels wide and 7 high
 * with its bottom row on the baseline (columns 2..5 of 10, rows 3..9 of 12 counted from the top),
 * B 3 wide and 5 high, 'AB' the two side by side with B starting 6 pixels after A; the ring O has
 * its hole. The images are printed row by row as two hex digits per pixel. */
#include "poly_mct.h"

static unsigned char *draw(const char *text, CGFloat size, CGFloat x, CGFloat y, int w, int h, const char *name) {
    unsigned char *px;
    CGContextRef c = pt_bitmap(w, h, 255, &px);
    CGContextSetShouldAntialias(c, 1);
    CGContextSetShouldSmoothFonts(c, 0);
    CGContextSetGrayFillColor(c, 0.0, 1.0);
    CGContextSetTextPosition(c, x, y);
    CTLineDraw(pt_line(text, pt_font(0, size)), c);
    value_str("image", name);
    pt_dump("p", px, w, h);
    return px;
}
static int box_is(const unsigned char *px, int w, int h, int x0, int y0, int x1, int y1) {
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++) {
            int inside = x >= x0 && x < x1 && y >= y0 && y < y1;
            if (px[y * w + x] != (inside ? 0x00 : 0xff)) return 0;
        }
    return 1;
}

int test_main(void) {
    unsigned char *a = draw("A", 10, 1, 2, 10, 12, "A");
    CK(box_is(a, 10, 12, 2, 3, 6, 10));
    unsigned char *b = draw("B", 10, 1, 2, 10, 10, "B");
    CK(box_is(b, 10, 10, 2, 3, 5, 8));
    unsigned char *ab = draw("AB", 10, 1, 2, 14, 12, "AB");
    int ok = 1;
    for (int y = 0; y < 12; y++)
        for (int x = 0; x < 14; x++) {
            int pa = x >= 2 && x < 6 && y >= 3 && y < 10, pb = x >= 8 && x < 11 && y >= 5 && y < 10;
            if (ab[y * 14 + x] != ((pa || pb) ? 0x00 : 0xff)) ok = 0;
        }
    CK(ok);
    unsigned char *o = draw("O", 10, 1, 2, 10, 12, "O");
    CK(o[3 * 10 + 1] == 0 && o[5 * 10 + 1] == 0 && o[5 * 10 + 3] == 0xff && o[7 * 10 + 5] == 0xff && o[7 * 10 + 7] == 0 && o[10 * 10 + 4] == 0xff);
    return finish();
}
