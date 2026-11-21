/* CGContextClipToRect: later drawing is limited to the rect; edges
 * antialiased for fractional rects; clips intersect; the clip is part of the
 * graphics state. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(8, 6);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextClipToRect(b.ctx, CGRectMake(2, 1, 4, 3));
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 8, 6));
    px_dump("whole-pixel clip (2,1,4,3) then a full fill", &b, 3);
    check("the clip box is the rect", CGRectEqualToRect(CGContextGetClipBoundingBox(b.ctx), CGRectMake(2, 1, 4, 3)), 0);

    pxbuf f = px_rgba(8, 6);
    CGContextSetRGBFillColor(f.ctx, 0, 0, 0, 1);
    CGContextClipToRect(f.ctx, CGRectMake(1.5, 1.25, 4.25, 3.5));
    CGContextFillRect(f.ctx, CGRectMake(0, 0, 8, 6));
    px_dump("fractional clip (1.5,1.25,4.25,3.5): antialiased edges", &f, 3);
    CGRect cb = CGContextGetClipBoundingBox(f.ctx);
    value_dbl("clip box x", cb.origin.x);
    value_dbl("clip box y", cb.origin.y);
    value_dbl("clip box w", cb.size.width);
    value_dbl("clip box h", cb.size.height);

    pxbuf i = px_rgba(8, 6);
    CGContextSetRGBFillColor(i.ctx, 0, 0, 0, 1);
    CGContextClipToRect(i.ctx, CGRectMake(0, 0, 5, 4));
    CGContextClipToRect(i.ctx, CGRectMake(3, 2, 5, 4));
    CGContextFillRect(i.ctx, CGRectMake(0, 0, 8, 6));
    px_dump("two clips intersect: (0,0,5,4) and (3,2,5,4)", &i, 3);

    pxbuf e = px_rgba(8, 6);
    CGContextSetRGBFillColor(e.ctx, 0, 0, 0, 1);
    CGContextClipToRect(e.ctx, CGRectMake(0, 0, 2, 2));
    CGContextClipToRect(e.ctx, CGRectMake(4, 4, 2, 2));
    CGContextFillRect(e.ctx, CGRectMake(0, 0, 8, 6));
    px_hash_value("disjoint clips: nothing is drawn (hash of an empty buffer)", &e);
    value("clip box of an empty clip: width", (long long)CGContextGetClipBoundingBox(e.ctx).size.width);
    return finish();
}
