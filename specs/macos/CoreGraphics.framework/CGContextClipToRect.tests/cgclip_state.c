/* Save and restore bring the old clip back; an empty rect clips everything
 * away; a clip is in user space at the time it is set (it does not follow
 * a later CTM change). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(8, 4);
    CGContextSetRGBFillColor(b.ctx, 0, 0, 0, 1);
    CGContextSaveGState(b.ctx);
    CGContextClipToRect(b.ctx, CGRectMake(0, 0, 2, 4));
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 8, 4));
    CGContextRestoreGState(b.ctx);
    CGContextClipToRect(b.ctx, CGRectMake(6, 0, 2, 4));
    CGContextFillRect(b.ctx, CGRectMake(0, 0, 8, 4));
    px_dump("a clip inside save/restore, then another clip, each followed by a full fill", &b, 3);

    pxbuf c = px_rgba(8, 4);
    CGContextSetRGBFillColor(c.ctx, 0, 0, 0, 1);
    CGContextClipToRect(c.ctx, CGRectMake(0, 0, 0, 4));
    CGContextFillRect(c.ctx, CGRectMake(0, 0, 8, 4));
    px_hash_value("an empty clip rect: nothing drawn", &c);

    pxbuf d = px_rgba(8, 4);
    CGContextSetRGBFillColor(d.ctx, 0, 0, 0, 1);
    CGContextClipToRect(d.ctx, CGRectMake(0, 0, 4, 4));
    CGContextTranslateCTM(d.ctx, 4, 0);
    CGContextFillRect(d.ctx, CGRectMake(0, 0, 8, 4));
    px_dump("a clip (0,0,4,4) then the CTM moved by 4: the fill is clipped in device space", &d, 3);
    return finish();
}
