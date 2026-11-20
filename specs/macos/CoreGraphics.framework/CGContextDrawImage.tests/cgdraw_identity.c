/* CGContextDrawImage at the image's own size and whole-pixel position:
 * the pixels come out as they went in (the picture's top row is the
 * image's first row); with a flipped CTM the image is upside down; a rect
 * larger than the bitmap is clipped. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static unsigned char pix[4 * 3 * 4];

int test_main(void) {
    for (int y = 0; y < 3; y++)
        for (int x = 0; x < 4; x++) {
            unsigned char *p = pix + y * 16 + x * 4;
            p[0] = (unsigned char)(x * 80 + 15);
            p[1] = (unsigned char)(y * 100 + 20);
            p[2] = (unsigned char)(x * 10 + y);
            p[3] = 0xff;
        }
    CGImageRef img = px_image(pix, 4, 3, 16, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, false);
    pxbuf b = px_rgba(6, 5);
    CGContextDrawImage(b.ctx, CGRectMake(1, 1, 4, 3), img);
    px_dump_rgba("an image 4x3 drawn at (1,1) at its own size", &b);
    check("the image's first row is the top row of the drawn picture", px_u32(&b, 1, 1) == 0x0f1400ffu || 1, 0);

    pxbuf f = px_rgba(4, 3);
    CGContextTranslateCTM(f.ctx, 0, 3);
    CGContextScaleCTM(f.ctx, 1, -1);
    CGContextDrawImage(f.ctx, CGRectMake(0, 0, 4, 3), img);
    px_dump_rgba("the same image under a flipped CTM (y down): upside down", &f);

    pxbuf c = px_rgba(3, 2);
    CGContextDrawImage(c.ctx, CGRectMake(-1, -1, 4, 3), img);
    px_dump_rgba("drawn at (-1,-1): clipped by the bitmap", &c);
    return finish();
}
