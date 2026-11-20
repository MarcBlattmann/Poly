/* The miter limit: a sharp corner is mitered up to the limit and bevelled
 * beyond it (default limit 10). Compare limits 1, 2, 4, 10 on a ~40 degree
 * corner (width 2), by the buffer hash and one pixel at the tip. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    double limits[] = {1, 2, 3, 4, 5.8, 6, 10, 20};
    for (unsigned i = 0; i < sizeof limits / sizeof *limits; i++) {
        pxbuf b = px_rgba(20, 12);
        CGContextSetRGBStrokeColor(b.ctx, 0, 0, 0, 1);
        CGContextSetLineWidth(b.ctx, 2);
        CGContextSetLineJoin(b.ctx, kCGLineJoinMiter);
        CGContextSetMiterLimit(b.ctx, (CGFloat)limits[i]);
        CGContextBeginPath(b.ctx);
        CGContextMoveToPoint(b.ctx, 1, 2);
        CGContextAddLineToPoint(b.ctx, 17, 6);
        CGContextAddLineToPoint(b.ctx, 1, 10);
        CGContextStrokePath(b.ctx);
        value("miter limit x10", (long long)(limits[i] * 10));
        px_hash_value("hash", &b);
        px_value("pixel at the tip (18, row 5)", &b, 18, 5);
    }
    pxbuf d = px_rgba(20, 12);
    CGContextSetRGBStrokeColor(d.ctx, 0, 0, 0, 1);
    CGContextSetLineWidth(d.ctx, 2);
    CGContextBeginPath(d.ctx);
    CGContextMoveToPoint(d.ctx, 1, 2);
    CGContextAddLineToPoint(d.ctx, 17, 6);
    CGContextAddLineToPoint(d.ctx, 1, 10);
    CGContextStrokePath(d.ctx);
    px_hash_value("default join and limit", &d);
    return finish();
}
