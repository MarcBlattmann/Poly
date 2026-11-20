/* The premultiplication rounding: colour component c at alpha a: the
 * stored byte for every a in steps of 17 and several c. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(1, 1);
    int comps[] = {255, 200, 128, 100, 51, 17, 3};
    for (unsigned ci = 0; ci < sizeof comps / sizeof *comps; ci++) {
        char row[200];
        size_t n = 0;
        static const char hex[] = "0123456789abcdef";
        for (int a = 0; a <= 255; a += 17) {
            px_clear(&b);
            CGContextSetRGBFillColor(b.ctx, comps[ci] / 255.0, comps[ci] / 255.0, comps[ci] / 255.0, a / 255.0);
            CGContextFillRect(b.ctx, CGRectMake(0, 0, 1, 1));
            row[n++] = hex[px_at(&b, 0, 0)[0] >> 4];
            row[n++] = hex[px_at(&b, 0, 0)[0] & 15];
            row[n++] = ' ';
        }
        row[n - 1] = 0;
        value("component (of 255)", comps[ci]);
        value_str("red byte for alpha 0, 17, ..., 255", row);
    }
    return finish();
}
