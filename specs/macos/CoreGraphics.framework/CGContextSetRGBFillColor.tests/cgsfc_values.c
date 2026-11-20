/* CGContextSetRGBFillColor: the bytes written for a grid of component
 * values, including out-of-range ones (clamped?) and the rounding of
 * components to 8 bits. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(1, 1);
    double v[] = {-0.5, 0, 1.0 / 510, 1.0 / 255 * 0.5, 0.5 / 255, 0.49 / 255, 1.0 / 255, 0.2, 0.3, 0.5, 127.5 / 255, 127.4 / 255, 0.7, 0.999, 1, 1.5};
    for (unsigned i = 0; i < sizeof v / sizeof *v; i++) {
        px_clear(&b);
        CGContextSetRGBFillColor(b.ctx, (CGFloat)v[i], (CGFloat)v[i], (CGFloat)v[i], 1);
        CGContextFillRect(b.ctx, CGRectMake(0, 0, 1, 1));
        value("component in 1/100000", (long long)(v[i] * 100000 + (v[i] < 0 ? -0.5 : 0.5)));
        px_value("opaque gray with that component", &b, 0, 0);
    }
    return finish();
}
