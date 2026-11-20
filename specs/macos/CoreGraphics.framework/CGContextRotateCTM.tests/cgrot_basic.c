/* CGContextRotateCTM: a rotated rect (alpha per pixel) for angles of a
 * quarter, an eighth and 30 degrees about a centre, and the CTM entries
 * (exact bits of cos/sin). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void rot(const char *name, double angle) {
    pxbuf b = px_rgba(12, 12);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextTranslateCTM(b.ctx, 6, 6);
    CGContextRotateCTM(b.ctx, (CGFloat)angle);
    CGContextFillRect(b.ctx, CGRectMake(-4, -2, 8, 4));
    px_dump(name, &b, 3);
    CGAffineTransform t = CGContextGetCTM(b.ctx);
    value_dbl("  CTM a", t.a);
    value_dbl("  CTM b", t.b);
    value_dbl("  CTM c", t.c);
    value_dbl("  CTM d", t.d);
}

int test_main(void) {
    const double pi = 3.14159265358979323846;
    rot("a rect 8x4 about (6,6), rotated pi/2", pi / 2);
    rot("rotated pi/4", pi / 4);
    rot("rotated pi/6 (30 degrees)", pi / 6);
    rot("rotated -pi/6", -pi / 6);
    rot("rotated pi", pi);
    return finish();
}
