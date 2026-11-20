/* Rect and size transforms through matrices (the rect becomes its bounding box). */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    const double pi = 3.14159265358979323846;
    CGRect r = CGRectApplyAffineTransform(CGRectMake(1, 2, 4, 2), CGAffineTransformMakeRotation((CGFloat)(pi / 2)));
    value_dbl("rect (1,2,4,2) rotated pi/2: x", r.origin.x);
    value_dbl("y", r.origin.y);
    value_dbl("w", r.size.width);
    value_dbl("h", r.size.height);
    CGRect s = CGRectApplyAffineTransform(CGRectMake(1, 2, 4, 2), CGAffineTransformMake(2, 0, 0, -1, 3, 0));
    value_dbl("rect (1,2,4,2) through (2 0 0 -1 3 0): x", s.origin.x);
    value_dbl("y", s.origin.y);
    value_dbl("w", s.size.width);
    value_dbl("h", s.size.height);
    CGRect inv = CGRectApplyAffineTransform(CGRectMake(1, 2, 4, 2), CGAffineTransformInvert(CGAffineTransformMake(2, 0, 0, 4, 5, 6)));
    value_dbl("through the inverse of (2 0 0 4 5 6): x", inv.origin.x);
    value_dbl("y", inv.origin.y);
    check("a null rect stays null", CGRectIsNull(CGRectApplyAffineTransform(CGRectNull, CGAffineTransformMakeScale(2, 2))), 0);
    return finish();
}
