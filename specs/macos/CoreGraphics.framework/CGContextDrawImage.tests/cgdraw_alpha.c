/* Drawing an image with alpha onto a coloured backdrop: source-over of
 * premultiplied pixels; the context alpha multiplies it; with a rotated CTM. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static unsigned char px4[4 * 4] = {0xff, 0x00, 0x00, 0xff, 0x80, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x40, 0x40};

int test_main(void) {
    CGImageRef img = px_image(px4, 4, 1, 16, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, false);
    pxbuf b = px_rgba(4, 1);
    px_backdrop(&b, 0, 0, 1, 1);
    CGContextDrawImage(b.ctx, CGRectMake(0, 0, 4, 1), img);
    px_dump_rgba("a row of 4 pixels (opaque red, half red, clear, quarter teal) over opaque blue", &b);
    pxbuf c = px_rgba(4, 1);
    px_backdrop(&c, 0, 0, 1, 1);
    CGContextSetAlpha(c.ctx, 0.5);
    CGContextDrawImage(c.ctx, CGRectMake(0, 0, 4, 1), img);
    px_dump_rgba("the same with the context alpha 0.5", &c);
    pxbuf d = px_rgba(4, 1);
    CGContextDrawImage(d.ctx, CGRectMake(0, 0, 4, 1), img);
    px_dump_rgba("onto a transparent backdrop: the image's own bytes", &d);
    return finish();
}
