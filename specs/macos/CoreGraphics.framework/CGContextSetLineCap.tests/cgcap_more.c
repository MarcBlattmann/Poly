/* Caps on a zero-length subpath (a dot) and on a closed path (none). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void dot(const char *name, int cap) {
    pxbuf b = px_rgba(8, 8);
    CGContextSetRGBStrokeColor(b.ctx, 0, 0, 0, 1);
    CGContextSetLineWidth(b.ctx, 4);
    CGContextSetLineCap(b.ctx, (CGLineCap)cap);
    CGContextBeginPath(b.ctx);
    CGContextMoveToPoint(b.ctx, 4, 4);
    CGContextAddLineToPoint(b.ctx, 4, 4);
    CGContextStrokePath(b.ctx);
    px_dump(name, &b, 3);
}

int test_main(void) {
    dot("a zero-length line with butt caps", kCGLineCapButt);
    dot("a zero-length line with round caps: a dot", kCGLineCapRound);
    dot("a zero-length line with square caps: a square", kCGLineCapSquare);
    return finish();
}
