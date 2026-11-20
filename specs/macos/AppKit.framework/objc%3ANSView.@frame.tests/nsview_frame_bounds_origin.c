/* poly-mac: appkit */
/* setBoundsOrigin: moves the view's coordinate system: a subview at (0,0) of a view whose bounds origin is
 * (5,5) is shown 5 points to the lower left (its position in the superview's coordinates is the frame
 * origin minus the bounds origin); the same for the conversion of the point (0,0) of the view
 * itself to the superview: it lands at frame origin minus bounds origin; the frame does not change;
 * setBounds: with a different origin and size sets both; translateOriginToPoint: is for drawing only
 * (not tested). */
#include "poly_mak.h"

int test_main(void) {
    NSView *sup = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 200, 200)];
    NSView *v = [[NSView alloc] initWithFrame:NSMakeRect(50, 60, 100, 100)];
    [sup addSubview:v];
    [v setBoundsOrigin:NSMakePoint(5, 5)];
    CK(NSEqualRects(v.frame, NSMakeRect(50, 60, 100, 100)));
    CK(NSEqualRects(v.bounds, NSMakeRect(5, 5, 100, 100)));
    NSPoint p = [v convertPoint:NSMakePoint(5, 5) toView:sup];
    CKF(p.x, 50);
    CKF(p.y, 60);
    p = [v convertPoint:NSMakePoint(0, 0) toView:sup];
    CKF(p.x, 45);
    CKF(p.y, 55);
    p = [sup convertPoint:NSMakePoint(50, 60) toView:v];
    CKF(p.x, 5);
    CKF(p.y, 5);
    NSView *child = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
    [v addSubview:child];
    p = [child convertPoint:NSMakePoint(0, 0) toView:sup];
    CKF(p.x, 45);
    [v setBounds:NSMakeRect(-10, -10, 50, 50)];
    CK(NSEqualRects(v.bounds, NSMakeRect(-10, -10, 50, 50)) && NSEqualRects(v.frame, NSMakeRect(50, 60, 100, 100)));
    p = [v convertPoint:NSMakePoint(-10, -10) toView:sup];
    CKF(p.x, 50);
    return finish();
}
