/* poly-mac: appkit */
/* init (without a frame) gives the zero frame and zero bounds; initWithFrame: with a zero size keeps
 * it; a negative width or height is standardized: the frame (5,7,-10,-20) becomes (-5,-13,10,20)
 * (origin moved by the negative size, size made positive) and the bounds are (0,0,10,20); `isEqual:` is
 * identity (two views with the same frame are not equal); the base class `tag` is -1 and
 * viewWithTag: of a view whose tag is -1 finds the view itself. */
#include "poly_mak.h"

int test_main(void) {
    NSView *z = [[NSView alloc] init];
    CK(NSEqualRects(z.frame, NSZeroRect) && NSEqualRects(z.bounds, NSZeroRect));
    NSView *a = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    NSView *b = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    CK(![a isEqual:b] && [a isEqual:a]);
    NSView *n = [[NSView alloc] initWithFrame:NSMakeRect(5, 7, -10, -20)];
    CK(NSEqualRects(n.frame, NSMakeRect(-5, -13, 10, 20)));
    CK(NSEqualRects(n.bounds, NSMakeRect(0, 0, 10, 20)));
    NSView *zs = [[NSView alloc] initWithFrame:NSMakeRect(5, 6, 0, 0)];
    CK(NSEqualRects(zs.frame, NSMakeRect(5, 6, 0, 0)));
    CK(a.tag == -1);
    [a addSubview:b];
    CK([a viewWithTag:-1] == a);
    return finish();
}
