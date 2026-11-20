/* A zero scale makes nothing visible; very large scales clip to the bitmap;
 * the CTM is reset by a restore. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf z = px_rgba(4, 4);
    CGContextSetRGBFillColor(z.ctx, 0, 0, 0, 1);
    CGContextScaleCTM(z.ctx, 0, 1);
    CGContextFillRect(z.ctx, CGRectMake(0, 0, 4, 4));
    px_hash_value("Scale(0, 1): nothing is drawn (empty buffer hash)", &z);
    pxbuf r = px_rgba(6, 4);
    CGContextSetRGBFillColor(r.ctx, 0, 0, 0, 1);
    CGContextSaveGState(r.ctx);
    CGContextScaleCTM(r.ctx, 3, 3);
    CGContextRestoreGState(r.ctx);
    CGContextFillRect(r.ctx, CGRectMake(0, 0, 2, 1));
    px_dump("the scale was undone by the restore", &r, 3);
    pxbuf big = px_rgba(6, 4);
    CGContextSetRGBFillColor(big.ctx, 0, 0, 0, 1);
    CGContextScaleCTM(big.ctx, 1000000, 1000000);
    CGContextFillRect(big.ctx, CGRectMake(0, 0, 1, 1));
    px_hash_value("Scale(1e6, 1e6) and a unit rect: covers the bitmap", &big);
    return finish();
}
