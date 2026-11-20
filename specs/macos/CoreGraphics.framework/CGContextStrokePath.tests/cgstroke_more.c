/* The stroke colour is separate from the fill colour; stroking an open path
 * with closed subpaths, and the default width, when asked after a flip. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(8, 4);
    CGContextSetRGBFillColor(b.ctx, 1, 0, 0, 1);
    CGContextSetRGBStrokeColor(b.ctx, 0, 0, 1, 1);
    CGContextSetLineWidth(b.ctx, 2);
    CGContextBeginPath(b.ctx);
    CGContextMoveToPoint(b.ctx, 1, 2);
    CGContextAddLineToPoint(b.ctx, 7, 2);
    CGContextStrokePath(b.ctx);
    px_dump_rgba("a stroke uses the stroke colour (blue), not the fill (red)", &b);
    pxbuf c = px_rgba(8, 8);
    CGContextSetRGBFillColor(c.ctx, 1, 0, 0, 1);
    CGContextSetRGBStrokeColor(c.ctx, 0, 0, 1, 1);
    CGContextSetLineWidth(c.ctx, 2);
    CGContextBeginPath(c.ctx);
    CGContextAddRect(c.ctx, CGRectMake(2, 2, 4, 4));
    CGContextDrawPath(c.ctx, kCGPathFillStroke);
    px_dump_rgba("fill and stroke of a rect, (2,2,4,4) width 2", &c);
    pxbuf d = px_rgba(8, 4);
    CGContextSetRGBStrokeColor(d.ctx, 0, 0, 0, 1);
    CGContextBeginPath(d.ctx);
    CGContextMoveToPoint(d.ctx, 1, 2);
    CGContextAddLineToPoint(d.ctx, 7, 2);
    CGContextStrokePath(d.ctx);
    px_dump("a stroke at the default line width (1) along y = 2: rows 1 and 2 share it", &d, 3);
    return finish();
}
