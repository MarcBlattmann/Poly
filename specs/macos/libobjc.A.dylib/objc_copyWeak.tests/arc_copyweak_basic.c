// poly-mac: libobjc
// objc_copyWeak: the destination (uninitialized) becomes a second weak reference to the same
// object; the source stays registered; neither retains; both are zeroed at the object's death.
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
    objc_copyWeak(&dst, &src);
    check("the destination holds the object", dst == o, 0);
    check("the source is unchanged", src == o, 0);
    value("retainCount unchanged", (i64)retain_count(o));
    objc_release(o);
    value("deallocs", deallocs);
    check("the source was zeroed", src == nil, 0);
    check("the destination was zeroed", dst == nil, 0);
    objc_destroyWeak(&src); objc_destroyWeak(&dst);
    return finish();
}
