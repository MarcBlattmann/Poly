/* poly-mac: coretext */
/* Edges between pixels get partial coverage: A at x position 1.5 has a half covered column on
 * each side (0x7f, within 1) and the black columns between; at size 7 (x 1 + 0.7 .. 1 + 3.5) the left
 * column is covered 0.3 (0xb3) and the right one half (0x7f); the vertical position is snapped to whole pixels: y 2.5 and 2 give the
 * same image; with antialiasing off (CGContextSetShouldAntialias 0) the pixels are only
 * black or white; the triangle V has intermediate gray values and is symmetric. Values within 2. */
#include "poly_mct.h"

static unsigned char *draw(const char *text, CGFloat size, CGFloat x, CGFloat y, int w, int h, int aa) {
    static unsigned char copies[8][4096];
    static int next;
    unsigned char *copy = copies[next++ % 8];
    unsigned char *px;
    CGContextRef c = pt_bitmap(w, h, 255, &px);
    CGContextSetShouldAntialias(c, aa);
    CGContextSetShouldSmoothFonts(c, 0);
    CGContextSetGrayFillColor(c, 0.0, 1.0);
    CGContextSetTextPosition(c, x, y);
    CTLineDraw(pt_line(text, pt_font(0, size)), c);
    for (int i = 0; i < w * h; i++) copy[i] = px[i];
    return copy;
}
static int within(int v, int want) { return v >= want - 2 && v <= want + 2; }

int test_main(void) {
    unsigned char *h = draw("A", 10, 1.5, 2, 10, 12, 1);
    CK(within(h[5 * 10 + 2], 0x7f) && h[5 * 10 + 3] == 0 && h[5 * 10 + 4] == 0 && h[5 * 10 + 5] == 0 && within(h[5 * 10 + 6], 0x7f));
    CK(h[5 * 10 + 1] == 0xff && h[5 * 10 + 7] == 0xff && h[2 * 10 + 3] == 0xff && h[10 * 10 + 3] == 0xff);
    unsigned char y25[120];
    unsigned char *a = draw("A", 10, 1, 2.5, 10, 12, 1);
    for (int i = 0; i < 120; i++) y25[i] = a[i];
    unsigned char *a2 = draw("A", 10, 1, 2, 10, 12, 1);
    int same = 1;
    for (int i = 0; i < 120; i++) same &= y25[i] == a2[i];
    CK(same);
    unsigned char *s = draw("A", 7, 1, 2, 10, 10, 1);
    CK(s[4 * 10 + 0] == 0xff && within(s[4 * 10 + 1], 0xb3) && s[4 * 10 + 2] == 0 && s[4 * 10 + 3] == 0 && within(s[4 * 10 + 4], 0x7f) && s[4 * 10 + 5] == 0xff);
    unsigned char *n = draw("A", 10, 1.5, 2, 10, 12, 0);
    int bw = 1;
    for (int i = 0; i < 120; i++) bw &= n[i] == 0 || n[i] == 0xff;
    CK(bw);
    unsigned char *v = draw("V", 10, 1, 2, 10, 12, 1);
    int gray = 0, sym = 1;
    for (int y = 0; y < 12; y++)
        for (int x = 0; x < 10; x++) {
            int p = v[y * 10 + x];
            if (p != 0 && p != 0xff) gray++;
        }
    for (int y = 0; y < 12; y++)
        for (int x = 0; x < 4; x++) sym &= within(v[y * 10 + (1 + x)], v[y * 10 + (1 + 6 - x)]);
    CK(gray > 5);
    CK(sym);
    return finish();
}
