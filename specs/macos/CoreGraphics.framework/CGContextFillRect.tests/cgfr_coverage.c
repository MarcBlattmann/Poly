/* The coverage rule of an antialiased rect edge: the alpha of the pixel
 * for coverage 0, 1/16, ..., 1 (opaque black); and the same for a
 * half-transparent colour. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(4, 4);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    char row[200];
    for (int k = 0; k <= 16; k++) {
        px_clear(&b);
        CGContextFillRect(b.ctx, CGRectMake(0, 0, k / 16.0, 4));
        static const char hex[] = "0123456789abcdef";
        row[k * 3] = hex[px_at(&b, 0, 1)[3] >> 4];
        row[k * 3 + 1] = hex[px_at(&b, 0, 1)[3] & 15];
        row[k * 3 + 2] = ' ';
    }
    row[17 * 3 - 1] = 0;
    value_str("alpha of the first column for a rect of width k/16, k = 0..16", row);
    for (int k = 0; k <= 16; k++) {
        px_clear(&b);
        CGContextFillRect(b.ctx, CGRectMake(k / 16.0, 0, 1 - k / 16.0, 4));
        static const char hex[] = "0123456789abcdef";
        row[k * 3] = hex[px_at(&b, 0, 1)[3] >> 4];
        row[k * 3 + 1] = hex[px_at(&b, 0, 1)[3] & 15];
        row[k * 3 + 2] = ' ';
    }
    row[17 * 3 - 1] = 0;
    value_str("alpha of the first column for a rect from k/16 to 1", row);
    /* coverage by area in two directions: a 0.5 x 0.5 corner is a quarter. */
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(0.5, 0.5, 0.5, 0.5));
    px_value("a 0.5 x 0.5 square in a pixel corner: a quarter of the pixel", &b, 0, 3);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(0.25, 0.25, 1.5, 1.5));
    px_dump("a 1.5 x 1.5 rect at (0.25, 0.25): alpha rows 0..3 (lower rows hold it)", &b, 3);
    CGContextSetRGBFillColor(b.ctx, 1, 0, 0, 0.5);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 0.5, 4));
    px_value("50% red at 50% coverage", &b, 0, 1);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 0.25, 4));
    px_value("50% red at 25% coverage", &b, 0, 1);
    return finish();
}
