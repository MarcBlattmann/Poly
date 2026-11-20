// poly-mac: libobjc
// poly-mac: arc
// objc_autoreleaseReturnValue as ARC-compiled code uses it: a callee (not inlined) returns a +0 new
// object, the caller keeps it in a strong variable. The result carries count 1 (no extra count from the
// handshake), nothing is left in the pool, and each object dies at the end of the caller's scope,
// before the pool is popped.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; }
@end
__attribute__((noinline)) static Obj *make(void) { return [Obj new]; }
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    {
        Obj *x = make();
        __unsafe_unretained Obj *u = x;
        value("retainCount of the returned object", (i64)retain_count(u));
        value("deallocs while it is held", deallocs);
    }
    value("deallocs at the end of the scope", deallocs);
    for (int i = 0; i < 100; i++) {
        Obj *y = make();
        (void)y;
    }
    value("deallocs after 100 more scopes, pool not yet popped", deallocs);
    objc_autoreleasePoolPop(p);
    value("deallocs after the pop", deallocs);
    return finish();
}
