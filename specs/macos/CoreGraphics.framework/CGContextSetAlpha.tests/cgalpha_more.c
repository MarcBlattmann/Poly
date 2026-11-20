/* Alpha on a stroke and on an edge pixel (coverage times alpha). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(6, 3);
    CGContextSetAlpha(b.ctx, 0.5);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextFillRect(b.ctx, CGRectMake(0.5, 0, 3, 3));
    px_dump("alpha 0.5, a rect with a half-covered left edge", &b, 3);
    pxbuf s = px_rgba(6, 3);
    CGContextSetAlpha(s.ctx, 0.5);
    CGContextSetRGBStrokeColor(s.ctx, 0, 0, 0, 1);
    CGContextSetLineWidth(s.ctx, 2);
    CGContextBeginPath(s.ctx);
    CGContextMoveToPoint(s.ctx, 0, 1.5);
    CGContextAddLineToPoint(s.ctx, 6, 1.5);
    CGContextStrokePath(s.ctx);
    px_dump("alpha 0.5, a width-2 stroke", &s, 3);
    return finish();
}
