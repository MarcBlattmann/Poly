/* Without antialiasing: width and height of a rect that does not start on a
 * pixel boundary: how many pixel rows and columns are painted, over a grid
 * of origins and sizes. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(8, 8);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextSetShouldAntialias(b.ctx, false);
    double xs[] = {0.0, 0.25, 0.49, 0.5, 0.51, 0.75};
    double ws[] = {0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.4, 2.5, 2.6};
    for (unsigned i = 0; i < sizeof xs / sizeof *xs; i++) {
        char row[200];
        size_t n = 0;
        for (unsigned j = 0; j < sizeof ws / sizeof *ws; j++) {
            px_clear(&b);
            CGContextFillRect(b.ctx, CGRectMake(1 + xs[i], 1, ws[j], 1));
            int count = 0;
            for (int x = 0; x < 8; x++) count += px_at(&b, x, 6)[3] != 0;
            row[n++] = (char)('0' + count);
            row[n++] = ' ';
        }
        row[n - 1] = 0;
        value("x offset in 1/100", (long long)(xs[i] * 100 + 0.5));
        value_str("columns painted for widths .25 .5 .75 1 1.25 1.5 2 2.4 2.5 2.6", row);
    }
    CGContextSetShouldAntialias(b.ctx, true);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(1.5, 1, 1.0, 1));
    px_dump("the same call with antialiasing on: 1.5, width 1", &b, 3);
    return finish();
}
