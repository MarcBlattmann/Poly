/* CGContextSetShouldAntialias(false): rect edges snap to whole pixels. The
 * rule: for a fill from x0 to x0 + 1, which pixel columns are painted, for
 * x0 over 0..1 in steps of 1/16 (a pixel is painted or not, never partial). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(4, 4);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextSetShouldAntialias(b.ctx, false);
    for (int k = 0; k <= 16; k++) {
        px_clear(&b);
        CGContextFillRect(b.ctx, CGRectMake(k / 16.0, 0, 1, 4));
        char row[8];
        for (int x = 0; x < 4; x++) row[x] = (char)('0' + (px_at(&b, x, 1)[3] ? 1 : 0));
        row[4] = ' ';
        row[5] = px_at(&b, 0, 1)[3] == 0 || px_at(&b, 0, 1)[3] == 255 ? 'o' : 'P';
        row[6] = 0;
        value("x0 in 1/16", k);
        value_str("painted columns (1 = painted), o = no partial alpha in col 0", row);
    }
    return finish();
}
