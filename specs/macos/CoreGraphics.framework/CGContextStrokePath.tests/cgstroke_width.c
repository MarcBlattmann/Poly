/* CGContextStrokePath of a horizontal and a diagonal line: alpha per pixel
 * for several widths and positions (butt caps, default join). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static void line(const char *name, double w, double x0, double y0, double x1, double y1) {
    pxbuf b = px_rgba(10, 6);
    CGContextSetRGBStrokeColor(b.ctx, 0, 0, 0, 1);
    CGContextSetLineWidth(b.ctx, (CGFloat)w);
    CGContextBeginPath(b.ctx);
    CGContextMoveToPoint(b.ctx, x0, y0);
    CGContextAddLineToPoint(b.ctx, x1, y1);
    CGContextStrokePath(b.ctx);
    px_dump(name, &b, 3);
}

int test_main(void) {
    line("width 1 horizontal on the pixel centre y = 2.5, x 1..9", 1, 1, 2.5, 9, 2.5);
    line("width 1 on a pixel edge y = 3, x 1..9", 1, 1, 3, 9, 3);
    line("width 2 on y = 3", 2, 1, 3, 9, 3);
    line("width 1.5 on y = 3", 1.5, 1, 3, 9, 3);
    line("width 0.5 on y = 2.5", 0.5, 1, 2.5, 9, 2.5);
    line("width 0 (a hairline?) on y = 2.5", 0, 1, 2.5, 9, 2.5);
    line("width 1 diagonal (1,1) to (9,5)", 1, 1, 1, 9, 5);
    line("width 2 diagonal (1,1) to (9,5)", 2, 1, 1, 9, 5);
    return finish();
}
