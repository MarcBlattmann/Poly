/* CGContextDrawLinearGradient: horizontal, vertical and diagonal axes;
 * without the extend options the gradient stops at the end points. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGFloat comps[8] = {1, 0, 0, 1, 0, 0, 1, 1};
    CGFloat locs[2] = {0, 1};
    CGGradientRef g = CGGradientCreateWithColorComponents(cs, comps, locs, 2);
    pxbuf h = px_rgba(8, 2);
    CGContextDrawLinearGradient(h.ctx, g, CGPointMake(0, 0), CGPointMake(8, 0), 0);
    px_dump_rgba("horizontal over the full width (rows 0 and 1)", &h);
    pxbuf v = px_rgba(2, 8);
    CGContextDrawLinearGradient(v.ctx, g, CGPointMake(0, 0), CGPointMake(0, 8), 0);
    px_dump_rgba("vertical from y = 0 (the bottom) to 8: red at the bottom", &v);
    pxbuf d = px_rgba(6, 6);
    CGContextDrawLinearGradient(d.ctx, g, CGPointMake(0, 0), CGPointMake(6, 6), 0);
    px_dump_rgba("diagonal from the lower left to the upper right", &d);
    pxbuf part = px_rgba(8, 1);
    CGContextDrawLinearGradient(part.ctx, g, CGPointMake(2, 0), CGPointMake(6, 0), 0);
    px_dump_rgba("end points 2..6 without extending: nothing outside", &part);
    return finish();
}
