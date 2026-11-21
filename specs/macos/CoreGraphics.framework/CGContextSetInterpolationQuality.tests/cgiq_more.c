/* The quality is part of the graphics state; the image's own
 * shouldInterpolate flag matters at the default quality only. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static unsigned char two[2 * 2 * 4] = {255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255};

int test_main(void) {
    pxbuf b = px_rgba(4, 4);
    CGContextSetInterpolationQuality(b.ctx, kCGInterpolationHigh);
    CGContextSaveGState(b.ctx);
    CGContextSetInterpolationQuality(b.ctx, kCGInterpolationNone);
    value("inside the save", CGContextGetInterpolationQuality(b.ctx));
    CGContextRestoreGState(b.ctx);
    value("after the restore", CGContextGetInterpolationQuality(b.ctx));
    CGImageRef plain = px_image(two, 2, 2, 8, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, false);
    CGImageRef smooth = px_image(two, 2, 2, 8, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, true);
    pxbuf p = px_rgba(4, 4), s = px_rgba(4, 4);
    CGContextDrawImage(p.ctx, CGRectMake(0, 0, 4, 4), plain);
    CGContextDrawImage(s.ctx, CGRectMake(0, 0, 4, 4), smooth);
    px_dump_rgba("default quality, shouldInterpolate false", &p);
    px_dump_rgba("default quality, shouldInterpolate true", &s);
    return finish();
}
