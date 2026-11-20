/* Clearing a bitmap without alpha: what remains (black or white?) */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    CGColorSpaceRef rgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    pxbuf b = px_make(2, 1, (CGBitmapInfo)kCGImageAlphaNoneSkipLast, rgb, 4, 8);
    CGContextSetRGBFillColor(b.ctx, 1, 0, 0, 1);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 2, 1));
    CGContextClearRect(b.ctx, CGRectMake(0, 0, 1, 1));
    px_dump_rgba("a bitmap without alpha (RGBX): cleared pixel then a red one", &b);
    CGColorSpaceRef gray = CGColorSpaceCreateDeviceGray();
    pxbuf g = px_make(2, 1, (CGBitmapInfo)kCGImageAlphaNone, gray, 1, 8);
    CGContextSetGrayFillColor(g.ctx, 0.5, 1);
    CGContextFillRect(g.ctx, CGRectMake(0, 0, 2, 1));
    CGContextClearRect(g.ctx, CGRectMake(0, 0, 1, 1));
    value("a gray bitmap without alpha, first pixel after the clear", g.data[0]);
    value("second", g.data[1]);
    return finish();
}
