/* Even-odd and nonzero winding: nested squares in the same and in the
 * opposite direction, and a star. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void squares(const char *name, int opposite, int eo) {
    pxbuf b = px_rgba(10, 10);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextBeginPath(b.ctx);
    CGContextMoveToPoint(b.ctx, 1, 1);
    CGContextAddLineToPoint(b.ctx, 9, 1);
    CGContextAddLineToPoint(b.ctx, 9, 9);
    CGContextAddLineToPoint(b.ctx, 1, 9);
    CGContextClosePath(b.ctx);
    CGContextMoveToPoint(b.ctx, 3, 3);
    if (opposite) {
        CGContextAddLineToPoint(b.ctx, 3, 7);
        CGContextAddLineToPoint(b.ctx, 7, 7);
        CGContextAddLineToPoint(b.ctx, 7, 3);
    } else {
        CGContextAddLineToPoint(b.ctx, 7, 3);
        CGContextAddLineToPoint(b.ctx, 7, 7);
        CGContextAddLineToPoint(b.ctx, 3, 7);
    }
    CGContextClosePath(b.ctx);
    if (eo) CGContextEOFillPath(b.ctx);
    else CGContextFillPath(b.ctx);
    px_dump(name, &b, 3);
}

int test_main(void) {
    squares("nonzero, same direction: solid", 0, 0);
    squares("nonzero, opposite direction: a frame", 1, 0);
    squares("even-odd, same direction: a frame", 0, 1);
    squares("even-odd, opposite direction: a frame", 1, 1);

    pxbuf s = px_rgba(12, 12);
    CGContextSetRGBFillColor(s.ctx, 0, 0, 0, 1);
    CGContextBeginPath(s.ctx);
    static const double pts[5][2] = {{6.0, 11.5}, {2.767181, 1.550407}, {11.230811, 7.699593}, {0.769189, 7.699593}, {9.232819, 1.550407}};
    for (int i = 0; i < 5; i++) {
        if (i == 0) CGContextMoveToPoint(s.ctx, pts[i][0], pts[i][1]);
        else CGContextAddLineToPoint(s.ctx, pts[i][0], pts[i][1]);
    }
    CGContextClosePath(s.ctx);
    CGContextEOFillPath(s.ctx);
    px_dump("a five-point star, even-odd (hole in the middle)", &s, 3);
    return finish();
}
