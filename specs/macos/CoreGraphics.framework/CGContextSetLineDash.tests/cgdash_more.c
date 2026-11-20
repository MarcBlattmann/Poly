/* Dashes with caps and around a corner; the dash pattern is part of the
 * graphics state; a zero or negative length makes no dash. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    CGFloat p[] = {3, 3};
    pxbuf b = px_rgba(12, 12);
    CGContextSetRGBStrokeColor(b.ctx, 0, 0, 0, 1);
    CGContextSetLineWidth(b.ctx, 2);
    CGContextSetLineDash(b.ctx, 0, p, 2);
    CGContextSetLineCap(b.ctx, kCGLineCapRound);
    CGContextBeginPath(b.ctx);
    CGContextMoveToPoint(b.ctx, 2, 2);
    CGContextAddLineToPoint(b.ctx, 10, 2);
    CGContextAddLineToPoint(b.ctx, 10, 10);
    CGContextStrokePath(b.ctx);
    px_dump("[3 3] with round caps around a corner", &b, 3);

    pxbuf c = px_rgba(12, 4);
    CGContextSetRGBStrokeColor(c.ctx, 0, 0, 0, 1);
    CGContextSetLineWidth(c.ctx, 2);
    CGContextSaveGState(c.ctx);
    CGContextSetLineDash(c.ctx, 0, p, 2);
    CGContextRestoreGState(c.ctx);
    CGContextBeginPath(c.ctx);
    CGContextMoveToPoint(c.ctx, 0, 2);
    CGContextAddLineToPoint(c.ctx, 12, 2);
    CGContextStrokePath(c.ctx);
    px_dump("the dash was restored away with the state: solid", &c, 3);

    CGFloat zero[] = {0, 3};
    pxbuf d = px_rgba(12, 4);
    CGContextSetRGBStrokeColor(d.ctx, 0, 0, 0, 1);
    CGContextSetLineWidth(d.ctx, 2);
    CGContextSetLineDash(d.ctx, 0, zero, 2);
    CGContextBeginPath(d.ctx);
    CGContextMoveToPoint(d.ctx, 0, 2);
    CGContextAddLineToPoint(d.ctx, 12, 2);
    CGContextStrokePath(d.ctx);
    px_dump("[0 3]: zero-length dashes with butt caps", &d, 3);
    return finish();
}
