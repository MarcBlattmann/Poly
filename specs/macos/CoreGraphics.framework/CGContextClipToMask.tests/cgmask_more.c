/* An RGBA bitmap's alpha as a mask (CGBitmapContextCreateImage of an alpha-only
 * context) and the mask drawn into a larger rect (scaled). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf a = px_make(4, 4, (CGBitmapInfo)kCGImageAlphaOnly, NULL, 1, 8);
    value("an alpha-only bitmap context is made (1) or not (0)", a.ctx != NULL);
    if (a.ctx) {
        CGContextSetGrayFillColor(a.ctx, 0, 0.5);
        CGContextFillEllipseInRect(a.ctx, CGRectMake(0.5, 0.5, 3, 3));
        CGImageRef mask = CGBitmapContextCreateImage(a.ctx);
        value("the image of an alpha-only context is a mask (1) or not (0)", CGImageIsMask(mask));
        pxbuf b = px_rgba(4, 4);
        CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
        CGContextClipToMask(b.ctx, CGRectMake(0, 0, 4, 4), mask);
        CGContextFillRect(b.ctx, CGRectMake(0, 0, 4, 4));
        px_dump("a black fill through the mask of a half-alpha circle", &b, 3);
    }
    return finish();
}
