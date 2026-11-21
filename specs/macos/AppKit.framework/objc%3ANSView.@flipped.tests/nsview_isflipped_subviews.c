/* poly-mac: appkit */
/* A subview's frame is in its superview's coordinates: in a flipped superview of height 100 a subview at
 * y 10 height 20 lies 10 points from the top; its (0,0) corner is at the bottom of an unflipped subview but
 * at the top of a flipped one, so converting its (0,0) to the superview gives y 30 for an
 * unflipped and 10 for a flipped subview; the same subview in an unflipped superview: y 10 for
 * the unflipped. The flippedness of the superview does not change the subview's own isFlipped. */
#include "poly_mak.h"

@interface FV : NSView
@end
@implementation FV
- (BOOL)isFlipped { return YES; }
@end

int test_main(void) {
    FV *fs = [[FV alloc] initWithFrame:NSMakeRect(0, 0, 100, 100)];
    NSView *su = [[NSView alloc] initWithFrame:NSMakeRect(5, 10, 30, 20)];
    FV *sf = [[FV alloc] initWithFrame:NSMakeRect(5, 10, 30, 20)];
    [fs addSubview:su];
    [fs addSubview:sf];
    CK(!su.isFlipped && sf.isFlipped);
    NSPoint p = [su convertPoint:NSMakePoint(0, 0) toView:fs];
    CKF(p.x, 5);
    CKF(p.y, 30);
    p = [sf convertPoint:NSMakePoint(0, 0) toView:fs];
    CKF(p.x, 5);
    CKF(p.y, 10);
    NSView *us = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, 100, 100)];
    NSView *su2 = [[NSView alloc] initWithFrame:NSMakeRect(5, 10, 30, 20)];
    [us addSubview:su2];
    p = [su2 convertPoint:NSMakePoint(0, 0) toView:us];
    CKF(p.y, 10);
    CK(!su2.isFlipped);
    NSPoint q = [fs convertPoint:NSMakePoint(5, 10) toView:su];
    value_f("flipped sup point in unflipped sub .y", q.y);
    return finish();
}
