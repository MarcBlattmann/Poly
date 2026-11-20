/* Drawn through a linear gradient across 17 pixels: the colour at each pixel
 * for gradients with stops at 0,1; 0,.5,1; unevenly placed; and gray. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void row(const char *name, CGGradientRef g, CGGradientDrawingOptions opts) {
    pxbuf b = px_rgba(17, 1);
    CGContextDrawLinearGradient(b.ctx, g, CGPointMake(0, 0), CGPointMake(17, 0), opts);
    px_dump_rgba(name, &b);
}

int test_main(void) {
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGFloat two[8] = {1, 0, 0, 1, 0, 0, 1, 1};
    CGFloat three[12] = {1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1, 1};
    CGFloat locs2[2] = {0, 1};
    CGFloat locs3[3] = {0, 0.5, 1};
    CGFloat locs3b[3] = {0, 0.2, 1};
    row("red to blue, locations 0 and 1", CGGradientCreateWithColorComponents(cs, two, locs2, 2), 0);
    row("red green blue, evenly spaced (NULL locations)", CGGradientCreateWithColorComponents(cs, three, NULL, 3), 0);
    row("red green blue at 0, 0.5, 1", CGGradientCreateWithColorComponents(cs, three, locs3, 3), 0);
    row("red green blue at 0, 0.2, 1", CGGradientCreateWithColorComponents(cs, three, locs3b, 3), 0);
    CGFloat alpha[8] = {0, 0, 0, 1, 0, 0, 0, 0};
    row("black opaque to black clear", CGGradientCreateWithColorComponents(cs, alpha, locs2, 2), 0);
    return finish();
}
