/* Premultiplied alpha: what is stored for a colour with alpha, and how
 * repeated draws accumulate. Observed per level of alpha. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    int alphas[] = {0, 1, 5, 10, 51, 64, 100, 127, 128, 150, 200, 254, 255};
    for (unsigned i = 0; i < sizeof alphas / sizeof *alphas; i++) {
        pxbuf b = px_rgba(1, 1);
        CGContextSetRGBFillColor(b.ctx, 1.0, 0.8, 0.4, alphas[i] / 255.0);
        CGContextFillRect(b.ctx, CGRectMake(0, 0, 1, 1));
        value("alpha of the fill colour (of 255)", alphas[i]);
        px_value("pixel (premultiplied RGBA)", &b, 0, 0);
    }
    pxbuf d = px_rgba(1, 1);
    CGContextSetRGBFillColor(d.ctx, 1, 0, 0, 0.5);
    for (int k = 1; k <= 5; k++) {
        CGContextFillRect(d.ctx, CGRectMake(0, 0, 1, 1));
        value("after this many 50% red fills", k);
        px_value("pixel", &d, 0, 0);
    }
    return finish();
}
