/* Joins inside a closed rect path and between curve segments; the default
 * miter limit is large enough for a right angle. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    int joins[] = {kCGLineJoinMiter, kCGLineJoinRound, kCGLineJoinBevel};
    for (int j = 0; j < 3; j++) {
        pxbuf b = px_rgba(12, 12);
        CGContextSetRGBStrokeColor(b.ctx, 0, 0, 0, 1);
        CGContextSetLineWidth(b.ctx, 3);
        CGContextSetLineJoin(b.ctx, (CGLineJoin)joins[j]);
        CGContextStrokeRect(b.ctx, CGRectMake(3, 3, 6, 6));
        value("join", joins[j]);
        px_dump("a stroked rect (3,3,6,6) width 3: row", &b, 3);
    }
    return finish();
}
