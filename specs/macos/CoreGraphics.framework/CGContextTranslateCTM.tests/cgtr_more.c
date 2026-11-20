/* ConcatCTM, SetCTM does not exist; GetUserSpaceToDeviceSpaceTransform and
 * conversions between user and device space. */
/* poly-mac: coregraphics */
#include "poly_mcg.h"

int test_main(void) {
    pxbuf b = px_rgba(10, 8);
    CGContextTranslateCTM(b.ctx, 3, 2);
    CGContextScaleCTM(b.ctx, 2, 2);
    CGAffineTransform t = CGContextGetUserSpaceToDeviceSpaceTransform(b.ctx);
    value_dbl("user to device a", t.a);
    value_dbl("user to device d", t.d);
    value_dbl("user to device tx", t.tx);
    value_dbl("user to device ty", t.ty);
    CGPoint p = CGContextConvertPointToDeviceSpace(b.ctx, CGPointMake(1, 1));
    value_dbl("(1,1) in device space x", p.x);
    value_dbl("(1,1) in device space y", p.y);
    CGPoint q = CGContextConvertPointToUserSpace(b.ctx, CGPointMake(9, 4));
    value_dbl("device (9,4) in user space x", q.x);
    value_dbl("device (9,4) in user space y", q.y);
    CGRect r = CGContextConvertRectToDeviceSpace(b.ctx, CGRectMake(1, 1, 2, 1));
    value_dbl("rect (1,1,2,1) in device space x", r.origin.x);
    value_dbl("y", r.origin.y);
    value_dbl("w", r.size.width);
    value_dbl("h", r.size.height);
    CGSize s = CGContextConvertSizeToDeviceSpace(b.ctx, CGSizeMake(1, 1));
    value_dbl("size (1,1) in device space w", s.width);
    value_dbl("h", s.height);
    return finish();
}
