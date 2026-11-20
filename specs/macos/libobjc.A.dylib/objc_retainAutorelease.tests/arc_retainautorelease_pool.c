// poly-mac: libobjc
// objc_retainAutorelease: the object outlives the owner's release until the pool is popped.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
static id passthrough(id o) { return objc_retainAutorelease(o); }
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    id o = [Obj new];
    id r = passthrough(o);
    objc_release(o);
    value("deallocs after the owner released", deallocs);
    value("retainCount", (i64)retain_count(r));
    objc_autoreleasePoolPop(p);
    value("deallocs after the pop", deallocs);
    // nested pool: autoreleased into the innermost pool
    void *q1 = objc_autoreleasePoolPush();
    id a = [Obj new];
    void *q2 = objc_autoreleasePoolPush();
    objc_retainAutorelease(a);
    objc_release(a);
    objc_autoreleasePoolPop(q2);
    value("deallocs after the inner pop", deallocs);
    objc_autoreleasePoolPop(q1);
    value("deallocs after the outer pop", deallocs);
    // no pool: leaks, count stays raised
    id b = [Obj new];
    objc_retainAutorelease(b);
    value("retainCount without a pool", (i64)retain_count(b));
    return finish();
}
