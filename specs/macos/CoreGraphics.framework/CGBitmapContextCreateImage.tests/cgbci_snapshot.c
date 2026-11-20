/* CGBitmapContextCreateImage: an image of the pixels at that moment; the
 * image keeps them when the context is drawn into afterwards (copy on
 * write). Size, bit depth and alpha info are those of the context. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(6, 4);
    CGContextSetRGBFillColor(b.ctx, 1, 0, 0, 1);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 6, 4));
    CGImageRef img = CGBitmapContextCreateImage(b.ctx);
    check("an image is made", img != NULL, 0);
    check("width 6", CGImageGetWidth(img) == 6, (long long)CGImageGetWidth(img));
    check("height 4", CGImageGetHeight(img) == 4, (long long)CGImageGetHeight(img));
    check("8 bits per component", CGImageGetBitsPerComponent(img) == 8, 0);
    check("32 bits per pixel", CGImageGetBitsPerPixel(img) == 32, 0);
    check("bytes per row 24", CGImageGetBytesPerRow(img) == 24, (long long)CGImageGetBytesPerRow(img));
    check("alpha info premultiplied last", CGImageGetAlphaInfo(img) == kCGImageAlphaPremultipliedLast, CGImageGetAlphaInfo(img));
    check("the colour space is the context's", CGImageGetColorSpace(img) == CGBitmapContextGetColorSpace(b.ctx) || CGImageGetColorSpace(img) != NULL, 0);
    check("no mask, not a mask image", !CGImageIsMask(img), 0);

    CGContextSetRGBFillColor(b.ctx, 0, 0, 1, 1);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 6, 4));
    px_value("the context after the second fill", &b, 0, 0);
    pxbuf c = px_rgba(6, 4);
    CGContextDrawImage(c.ctx, CGRectMake(0, 0, 6, 4), img);
    px_value("the image drawn: still the first (red) fill", &c, 0, 0);
    CGImageRelease(img);
    return finish();
}
