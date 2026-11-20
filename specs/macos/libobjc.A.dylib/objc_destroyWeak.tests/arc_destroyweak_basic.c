// poly-mac: libobjc
// objc_destroyWeak: unregisters a weak location; the object's deallocation no longer touches it,
// the storage can be reused for anything.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id loc = 0;
    objc_initWeak(&loc, o);
    objc_destroyWeak(&loc);
    value("retainCount unchanged by destroyWeak", (i64)retain_count(o));
    value("deallocs", deallocs);
    // the storage now holds a plain value: the object's death must leave it alone
    loc = (id)0x1234;
    objc_release(o);
    value("deallocs after release", deallocs);
    check("the storage was not written by the object's dealloc", loc == (id)0x1234, 0);
    // a nil location
    id nl = 0;
    objc_initWeak(&nl, nil);
    objc_destroyWeak(&nl);
    value("destroyWeak on a nil weak location survived", 1);
    // the other registered location of the same object stays registered
    id p = [Obj new]; id l1 = 0, l2 = 0;
    objc_initWeak(&l1, p); objc_initWeak(&l2, p);
    objc_destroyWeak(&l1);
    l1 = (id)0x4321;
    objc_release(p);
    check("the remaining location is zeroed", l2 == nil, 0);
    check("the destroyed one is left alone", l1 == (id)0x4321, 0);
    return finish();
}
