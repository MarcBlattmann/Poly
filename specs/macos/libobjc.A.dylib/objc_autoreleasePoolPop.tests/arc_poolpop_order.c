// poly-mac: libobjc
// objc_autoreleasePoolPop: releases the pool's objects newest first, inner pools before outer
// ones, and an object still pending in an outer pool is not touched.
#include "poly_mobjc.h"
static int log_[16], n;
@interface Obj : NSObject { @public int tag; } @end
@implementation Obj
- (void)dealloc { if (n < 16) log_[n++] = tag; [super dealloc]; }
@end
static void mk(int t) { Obj *o = [Obj new]; o->tag = t; objc_autorelease((id)o); }
int test_main(void) {
    void *p1 = objc_autoreleasePoolPush();
    mk(1); mk(2);
    void *p2 = objc_autoreleasePoolPush();
    mk(3); mk(4);
    objc_autoreleasePoolPop(p2);
    value("events after the inner pop", n);
    for (int i = 0; i < n; i++) value("event", log_[i]);
    mk(5);
    objc_autoreleasePoolPop(p1);
    value("events after the outer pop", n);
    for (int i = 4; i < n; i++) value("event", log_[i]);
    return finish();
}
