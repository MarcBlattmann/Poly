/* CGContextFillEllipseInRect: the alpha of every pixel of antialiased
 * circles of several diameters and positions (opaque black); the exact
 * coverage record. Rows are from the top of the bitmap. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void circle(const char *name, size_t n, double x, double y, double w, double h) {
    pxbuf b = px_rgba(n, n);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextFillEllipseInRect(b.ctx, CGRectMake(x, y, w, h));
    px_dump(name, &b, 3);
}

int test_main(void) {
    circle("diameter 2 at (1,1) in 4x4", 4, 1, 1, 2, 2);
    circle("diameter 3 at (1,1) in 5x5", 5, 1, 1, 3, 3);
    circle("diameter 4 at (1,1) in 6x6", 6, 1, 1, 4, 4);
    circle("diameter 5 at (1,1) in 7x7", 7, 1, 1, 5, 5);
    circle("diameter 6 at (1,1) in 8x8", 8, 1, 1, 6, 6);
    circle("diameter 8 at (1,1) in 10x10", 10, 1, 1, 8, 8);
    circle("diameter 7 at (0.5, 0.5) in 9x9 (half-pixel origin)", 9, 0.5, 0.5, 7, 7);
    circle("diameter 6.4 at (1.3, 0.8) in 9x9", 9, 1.3, 0.8, 6.4, 6.4);
    return finish();
}
