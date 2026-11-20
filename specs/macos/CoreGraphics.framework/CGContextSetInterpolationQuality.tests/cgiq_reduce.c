/* A 8x8 checkerboard of white and black reduced to 2x2 and 3x3 at each
 * quality (none picks pixels, the others filter). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

static unsigned char board[8 * 8 * 4];

int test_main(void) {
    for (int y = 0; y < 8; y++)
        for (int x = 0; x < 8; x++) {
            unsigned char v = (unsigned char)(((x + y) & 1) ? 255 : 0);
            unsigned char *p = board + (y * 8 + x) * 4;
            p[0] = p[1] = p[2] = v;
            p[3] = 255;
        }
    CGImageRef img = px_image(board, 8, 8, 32, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, false);
    int qs[] = {kCGInterpolationNone, kCGInterpolationLow, kCGInterpolationMedium, kCGInterpolationHigh};
    const char *names[] = {"none", "low", "medium", "high"};
    for (int i = 0; i < 4; i++) {
        pxbuf b = px_rgba(3, 3);
        CGContextSetInterpolationQuality(b.ctx, (CGInterpolationQuality)qs[i]);
        CGContextDrawImage(b.ctx, CGRectMake(0, 0, 3, 3), img);
        px_dump_rgba(names[i], &b);
    }
    return finish();
}
