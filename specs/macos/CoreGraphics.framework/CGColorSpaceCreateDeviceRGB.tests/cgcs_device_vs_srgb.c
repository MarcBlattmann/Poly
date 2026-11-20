/* The same fill in a device RGB bitmap and in an sRGB bitmap: bytes written
 * for a range of colours (observed, not derived). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    CGColorSpaceRef dev = CGColorSpaceCreateDeviceRGB();
    CGColorSpaceRef srgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    double levels[] = {0, 0.1, 0.25, 0.5, 0.75, 0.9, 1};
    int diff = 0;
    for (unsigned i = 0; i < sizeof levels / sizeof *levels; i++) {
        pxbuf a = px_make(1, 1, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, dev, 4, 8);
        pxbuf c = px_make(1, 1, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, srgb, 4, 8);
        CGContextSetRGBFillColor(a.ctx, (CGFloat)levels[i], (CGFloat)levels[i], (CGFloat)levels[i], 1);
        CGContextSetRGBFillColor(c.ctx, (CGFloat)levels[i], (CGFloat)levels[i], (CGFloat)levels[i], 1);
        CGContextFillRect(a.ctx, CGRectMake(0, 0, 1, 1));
        CGContextFillRect(c.ctx, CGRectMake(0, 0, 1, 1));
        if (px_u32(&a, 0, 0) != px_u32(&c, 0, 0)) diff++;
        value("level in 1/100", (long long)(levels[i] * 100 + 0.5));
        px_value("device RGB bitmap", &a, 0, 0);
        px_value("sRGB bitmap", &c, 0, 0);
    }
    value("levels where the two bitmaps differ", diff);
    return finish();
}
