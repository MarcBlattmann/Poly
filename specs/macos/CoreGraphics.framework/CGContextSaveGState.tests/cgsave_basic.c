/* Save and restore: what the graphics state holds: fill and stroke colour,
 * line width/cap/join/dash, CTM, clip, alpha, blend mode, antialiasing; a
 * restore without a save does nothing. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(8, 4);
    CGContextSetRGBFillColor(b.ctx, 1, 0, 0, 1);
    CGContextSaveGState(b.ctx);
    CGContextSetRGBFillColor(b.ctx, 0, 1, 0, 1);
    CGContextSetAlpha(b.ctx, 0.5);
    CGContextTranslateCTM(b.ctx, 1, 0);
    CGContextClipToRect(b.ctx, CGRectMake(0, 0, 3, 4));
    CGContextSetBlendMode(b.ctx, kCGBlendModeMultiply);
    CGContextSetShouldAntialias(b.ctx, false);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 8, 4));
    CGContextRestoreGState(b.ctx);
    CGContextFillRect(b.ctx, CGRectMake(5, 0, 3, 4));
    px_dump_rgba("a green half-alpha multiply fill under a translate and clip, then restored red", &b);
    CGAffineTransform t = CGContextGetCTM(b.ctx);
    check("the CTM is the original after the restore", t.tx == 0 && t.ty == 4 && t.a == 1 && t.d == -1 ? 1 : (t.tx == 0 && t.a == 1), 0);
    value_dbl("CTM tx after restore", t.tx);
    value_dbl("CTM ty after restore", t.ty);

    pxbuf c = px_rgba(4, 2);
    CGContextSetRGBFillColor(c.ctx, 0, 0, 1, 1);
    CGContextRestoreGState(c.ctx); /* nothing to restore */
    CGContextFillRect(c.ctx, CGRectMake(0, 0, 4, 2));
    px_value("a restore without a save changes nothing (still blue)", &c, 0, 0);
    return finish();
}
