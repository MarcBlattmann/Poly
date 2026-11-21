// poly-mac: libobjc
// objc_moveWeak: the destination (uninitialized) becomes the weak reference, the source is left
// zeroed and unregistered; no count changes; the object's death zeroes the destination.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id src = 0;
    objc_initWeak(&src, o);
    id dst = (id)0x1;
    objc_moveWeak(&dst, &src);
    check("the destination holds the object", dst == o, 0);
    value("the source is nil afterwards", src == nil);
    value("retainCount unchanged", (i64)retain_count(o));
    objc_release(o);
    value("deallocs", deallocs);
    check("the destination was zeroed at the death", dst == nil, 0);
    objc_destroyWeak(&dst);
    return finish();
}
