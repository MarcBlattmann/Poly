/* CGContextFillRect on whole-pixel rects: exact bytes, the clipping at the
 * edges of the bitmap, empty and negative rects. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(8, 6);
    CGContextSetRGBFillColor(b.ctx, 1, 0.5, 0.25, 1);
    CGContextFillRect(b.ctx, CGRectMake(2, 1, 4, 3));
    px_dump_rgba("rect (2,1,4,3) in an 8x6 bitmap; rows from the top", &b);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(-3, -3, 6, 6));
    px_hash_value("hash", &b);
    px_value("pixel (0, last row)", &b, 0, 5);
    px_value("pixel (3, last row): just outside the rect (x from -3 to 3)", &b, 3, 5);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(5, 4, 100, 100));
    px_hash_value("a rect far past the upper right corner", &b);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(1, 1, 0, 3));
    px_hash_value("a rect of width 0 draws nothing (hash of an empty buffer)", &b);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectMake(5, 4, -3, -3));
    px_hash_value("a rect with negative width and height: the same as the standardised rect (2,1,3,3)", &b);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectNull);
    px_hash_value("CGRectNull draws nothing", &b);
    px_clear(&b);
    CGContextFillRect(b.ctx, CGRectInfinite);
    px_hash_value("CGRectInfinite fills everything", &b);
    check("every pixel of the bitmap is set by the infinite rect", px_u32(&b, 0, 0) == 0xff8040ffu && px_u32(&b, 7, 5) == 0xff8040ffu, 0);
    return finish();
}
