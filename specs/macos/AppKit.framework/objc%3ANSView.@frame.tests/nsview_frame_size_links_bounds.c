/* poly-mac: appkit */
/* The frame and the bounds of a view are linked: setFrameSize: changes the bounds size to the same size
 * (while the bounds origin stays); setFrameOrigin: does not change the bounds; after setBoundsSize: the bounds are
 * what was set and the frame does not change (the view is scaled: a point of the bounds is converted to
 * the superview by the factor frame size / bounds size); a later setFrameSize: keeps the scale. */
#include "poly_mak.h"

int test_main(void) {
    NSView *sup = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 400, 400)];
    NSView *v = [[NSView alloc] initWithFrame:NSMakeRect(10, 20, 100, 50)];
    [sup addSubview:v];
    [v setFrameSize:NSMakeSize(200, 100)];
    CK(NSEqualRects(v.bounds, NSMakeRect(0, 0, 200, 100)));
    [v setFrameOrigin:NSMakePoint(7, 8)];
    CK(NSEqualRects(v.bounds, NSMakeRect(0, 0, 200, 100)) && NSEqualRects(v.frame, NSMakeRect(7, 8, 200, 100)));
    [v setBoundsSize:NSMakeSize(100, 50)];
    CK(NSEqualRects(v.frame, NSMakeRect(7, 8, 200, 100)));
    CK(NSEqualRects(v.bounds, NSMakeRect(0, 0, 100, 50)));
    NSPoint p = [v convertPoint:NSMakePoint(10, 10) toView:sup];
    CKF(p.x, 7 + 20);
    CKF(p.y, 8 + 20);
    [v setFrameSize:NSMakeSize(400, 200)];
    CKF(v.bounds.size.width, 200);
    CKF(v.bounds.size.height, 100);
    value_f("frame.w", v.frame.size.width);
    return finish();
}
