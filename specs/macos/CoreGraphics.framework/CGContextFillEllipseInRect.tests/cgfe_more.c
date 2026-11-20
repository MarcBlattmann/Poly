/* An ellipse is four cubic curves: the same picture as a path of four
 * curves with the usual control points, and as CGPathAddEllipseInRect;
 * with antialiasing off the covered pixels. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf a = px_rgba(12, 12);
    CGContextSetRGBFillColor(a.ctx, 0, 0, 0, 1);
    CGContextFillEllipseInRect(a.ctx, CGRectMake(1, 1, 10, 10));
    pxbuf p = px_rgba(12, 12);
    CGContextSetRGBFillColor(p.ctx, 0, 0, 0, 1);
    CGMutablePathRef path = CGPathCreateMutable();
    CGPathAddEllipseInRect(path, NULL, CGRectMake(1, 1, 10, 10));
    CGContextAddPath(p.ctx, path);
    CGContextFillPath(p.ctx);
    check("FillEllipseInRect equals filling CGPathAddEllipseInRect", px_hash(&a) == px_hash(&p), 0);
    px_hash_value("hash of the 10x10 circle", &a);

    pxbuf c = px_rgba(12, 12);
    CGContextSetRGBFillColor(c.ctx, 0, 0, 0, 1);
    const double k = 0.5522847498307935; /* 4/3 (sqrt 2 - 1) */
    double cx = 6, cy = 6, r = 5;
    CGContextBeginPath(c.ctx);
    CGContextMoveToPoint(c.ctx, cx + r, cy);
    CGContextAddCurveToPoint(c.ctx, cx + r, cy + k * r, cx + k * r, cy + r, cx, cy + r);
    CGContextAddCurveToPoint(c.ctx, cx - k * r, cy + r, cx - r, cy + k * r, cx - r, cy);
    CGContextAddCurveToPoint(c.ctx, cx - r, cy - k * r, cx - k * r, cy - r, cx, cy - r);
    CGContextAddCurveToPoint(c.ctx, cx + k * r, cy - r, cx + r, cy - k * r, cx + r, cy);
    CGContextClosePath(c.ctx);
    CGContextFillPath(c.ctx);
    value("the four-curve path equals the ellipse call (1 = identical bytes)", px_hash(&a) == px_hash(&c));
    px_dump("four curves, kappa 0.5522847498307935", &c, 3);

    pxbuf off = px_rgba(12, 12);
    CGContextSetRGBFillColor(off.ctx, 0, 0, 0, 1);
    CGContextSetShouldAntialias(off.ctx, false);
    CGContextFillEllipseInRect(off.ctx, CGRectMake(1, 1, 10, 10));
    px_dump("the 10x10 circle without antialiasing", &off, 3);
    return finish();
}
