/* The default orientation: the origin is the lower left, y goes up, and
 * row 0 of the buffer is the top of the picture; CGContextScaleCTM(1, -1)
 * with a translate flips it. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(4, 4);
    CGContextSetRGBFillColor(b.ctx, 1, 0, 0, 1);
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 1, 1));
    CGContextSetRGBFillColor(b.ctx, 0, 1, 0, 1);
    CGContextFillRect(b.ctx, CGRectMake(3, 3, 1, 1));
    px_dump_rgba("origin at the lower left", &b);
    check("(0,0) in user space is the buffer's last row, first pixel", px_u32(&b, 0, 3) == 0xff0000ff, (long long)px_u32(&b, 0, 3));
    check("(3,3) is row 0, last pixel", px_u32(&b, 3, 0) == 0x00ff00ff, (long long)px_u32(&b, 3, 0));

    pxbuf f = px_rgba(4, 4);
    CGContextTranslateCTM(f.ctx, 0, 4);
    CGContextScaleCTM(f.ctx, 1, -1);
    CGContextSetRGBFillColor(f.ctx, 1, 0, 0, 1);
    CGContextFillRect(f.ctx, CGRectMake(0, 0, 1, 1));
    px_dump_rgba("flipped (y down)", &f);
    check("after the flip (0,0) is row 0, first pixel", px_u32(&f, 0, 0) == 0xff0000ff, (long long)px_u32(&f, 0, 0));
    CGAffineTransform t = CGContextGetCTM(f.ctx);
    value_dbl("CTM a", t.a); value_dbl("CTM d", t.d); value_dbl("CTM ty", t.ty);
    return finish();
}
