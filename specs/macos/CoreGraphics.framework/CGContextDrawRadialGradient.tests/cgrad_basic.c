/* CGContextDrawRadialGradient: concentric circles (start radius 0, end
 * radius 5), with and without the extend options, and an off-centre start. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGFloat comps[8] = {1, 1, 1, 1, 0, 0, 0, 1};
    CGFloat locs[2] = {0, 1};
    CGGradientRef g = CGGradientCreateWithColorComponents(cs, comps, locs, 2);
    pxbuf a = px_rgba(12, 12);
    CGContextDrawRadialGradient(a.ctx, g, CGPointMake(6, 6), 0, CGPointMake(6, 6), 5, 0);
    px_dump("concentric 0..5, no extension: the red channel", &a, 0);
    px_dump("alpha (outside the end circle: nothing)", &a, 3);
    pxbuf b = px_rgba(12, 12);
    CGContextDrawRadialGradient(b.ctx, g, CGPointMake(6, 6), 0, CGPointMake(6, 6), 5, kCGGradientDrawsAfterEndLocation);
    px_dump("with the extension after the end: the red channel", &b, 0);
    pxbuf c = px_rgba(12, 12);
    CGContextDrawRadialGradient(c.ctx, g, CGPointMake(4, 6), 0, CGPointMake(7, 6), 5, 0);
    px_dump("off-centre: start (4,6) radius 0, end (7,6) radius 5: the red channel", &c, 0);
    return finish();
}
