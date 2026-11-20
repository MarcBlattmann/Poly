/* Gradients in other colour spaces drawn into sRGB. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    CGFloat locs[2] = {0, 1};
    CGColorSpaceRef gray = CGColorSpaceCreateDeviceGray();
    CGFloat gc[4] = {0, 1, 1, 1};
    CGGradientRef g = CGGradientCreateWithColorComponents(gray, gc, locs, 2);
    pxbuf b = px_rgba(9, 1);
    CGContextDrawLinearGradient(b.ctx, g, CGPointMake(0, 0), CGPointMake(9, 0), 0);
    px_dump_rgba("device gray black to white", &b);
    CGColorSpaceRef p3 = CGColorSpaceCreateWithName(kCGColorSpaceDisplayP3);
    CGFloat pc[8] = {1, 0, 0, 1, 0, 1, 0, 1};
    CGGradientRef h = CGGradientCreateWithColorComponents(p3, pc, locs, 2);
    pxbuf c = px_rgba(9, 1);
    CGContextDrawLinearGradient(c.ctx, h, CGPointMake(0, 0), CGPointMake(9, 0), 0);
    px_dump_rgba("Display P3 red to green into an sRGB bitmap", &c);
    return finish();
}
