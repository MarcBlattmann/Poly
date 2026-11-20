/* The extend options: kCGGradientDrawsBeforeStartLocation (1) and
 * AfterEndLocation (2). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGFloat comps[8] = {1, 0, 0, 1, 0, 0, 1, 1};
    CGFloat locs[2] = {0, 1};
    CGGradientRef g = CGGradientCreateWithColorComponents(cs, comps, locs, 2);
    value("kCGGradientDrawsBeforeStartLocation", kCGGradientDrawsBeforeStartLocation);
    value("kCGGradientDrawsAfterEndLocation", kCGGradientDrawsAfterEndLocation);
    int opts[] = {0, kCGGradientDrawsBeforeStartLocation, kCGGradientDrawsAfterEndLocation, kCGGradientDrawsBeforeStartLocation | kCGGradientDrawsAfterEndLocation};
    const char *names[] = {"no extension", "before start", "after end", "both"};
    for (int i = 0; i < 4; i++) {
        pxbuf b = px_rgba(10, 1);
        CGContextDrawLinearGradient(b.ctx, g, CGPointMake(3, 0), CGPointMake(7, 0), (CGGradientDrawingOptions)opts[i]);
        px_dump_rgba(names[i], &b);
    }
    return finish();
}
