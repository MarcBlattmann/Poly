/* Blend modes with partial coverage at an edge, and with an image. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static unsigned char px1[4] = {0x40, 0x80, 0xc0, 0xff};

int test_main(void) {
    int modes[] = {kCGBlendModeMultiply, kCGBlendModeScreen, kCGBlendModeCopy, kCGBlendModeClear, kCGBlendModeSourceIn, kCGBlendModeDestinationOut};
    const char *names[] = {"multiply", "screen", "copy", "clear", "source in", "destination out"};
    for (int i = 0; i < 6; i++) {
        pxbuf b = px_rgba(3, 1);
        px_backdrop(&b, 0.8, 0.4, 0.2, 1);
        CGContextSetBlendMode(b.ctx, (CGBlendMode)modes[i]);
        CGContextSetRGBFillColor(b.ctx, 0.3, 0.6, 0.9, 1);
        CGContextFillRect(b.ctx, CGRectMake(0.5, 0, 1.5, 1));
        px_dump_rgba(names[i], &b);
    }
    CGImageRef img = px_image(px1, 1, 1, 4, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, false);
    pxbuf d = px_rgba(1, 1);
    px_backdrop(&d, 0.8, 0.4, 0.2, 1);
    CGContextSetBlendMode(d.ctx, kCGBlendModeMultiply);
    CGContextDrawImage(d.ctx, CGRectMake(0, 0, 1, 1), img);
    px_value("an image drawn with multiply", &d, 0, 0);
    return finish();
}
