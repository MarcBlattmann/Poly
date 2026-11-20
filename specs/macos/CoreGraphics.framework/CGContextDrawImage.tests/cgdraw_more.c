/* Drawing the same image through a clip, a rotation of a quarter turn
 * (exact for pixel-aligned rects), and into a rect with negative size
 * (mirrored). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static unsigned char two[2 * 2 * 4] = {255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255};

int test_main(void) {
    const double pi = 3.14159265358979323846;
    CGImageRef img = px_image(two, 2, 2, 8, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, false);
    pxbuf q = px_rgba(2, 2);
    CGContextTranslateCTM(q.ctx, 1, 1);
    CGContextRotateCTM(q.ctx, (CGFloat)(pi / 2));
    CGContextDrawImage(q.ctx, CGRectMake(-1, -1, 2, 2), img);
    px_dump_rgba("rotated a quarter turn about the centre", &q);
    pxbuf m = px_rgba(2, 2);
    CGContextDrawImage(m.ctx, CGRectMake(2, 0, -2, 2), img);
    px_dump_rgba("a rect with negative width (2,0,-2,2): the image is standardised, not mirrored?", &m);
    pxbuf c = px_rgba(2, 2);
    CGContextClipToRect(c.ctx, CGRectMake(1, 0, 1, 2));
    CGContextDrawImage(c.ctx, CGRectMake(0, 0, 2, 2), img);
    px_dump_rgba("through a clip of the right column", &c);
    return finish();
}
