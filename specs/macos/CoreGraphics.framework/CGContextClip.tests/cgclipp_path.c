/* CGContextClip with a path: an antialiased clip from a circle and a
 * triangle; CGContextEOClip on two nested rects; the path is consumed. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(10, 10);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextBeginPath(b.ctx);
    CGContextAddEllipseInRect(b.ctx, CGRectMake(1, 1, 8, 8));
    CGContextClip(b.ctx);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 10, 10));
    px_dump("a fill clipped by a circle (1,1,8,8)", &b, 3);

    pxbuf c = px_rgba(10, 10);
    CGContextSetRGBFillColor(c.ctx, 0, 0, 0, 1);
    CGContextBeginPath(c.ctx);
    CGContextAddRect(c.ctx, CGRectMake(1, 1, 8, 8));
    CGContextAddRect(c.ctx, CGRectMake(3, 3, 4, 4));
    CGContextEOClip(c.ctx);
    CGContextFillRect(c.ctx, CGRectMake(0, 0, 10, 10));
    px_dump("EOClip of nested rects: a frame", &c, 3);

    pxbuf d = px_rgba(10, 10);
    CGContextSetRGBFillColor(d.ctx, 0, 0, 0, 1);
    CGContextBeginPath(d.ctx);
    CGContextAddRect(d.ctx, CGRectMake(1, 1, 8, 8));
    CGContextAddRect(d.ctx, CGRectMake(3, 3, 4, 4));
    CGContextClip(d.ctx);
    CGContextFillRect(d.ctx, CGRectMake(0, 0, 10, 10));
    px_dump("Clip (nonzero) of the same nested rects (same direction): solid", &d, 3);
    check("after Clip the current path is empty", CGContextIsPathEmpty(d.ctx), 0);
    return finish();
}
