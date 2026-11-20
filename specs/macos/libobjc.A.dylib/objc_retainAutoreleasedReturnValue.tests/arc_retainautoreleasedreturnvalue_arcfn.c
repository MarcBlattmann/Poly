// poly-mac: libobjc
// poly-mac: arc
// objc_retainAutoreleasedReturnValue as ARC-compiled code uses it: the caller keeps the result of a
// call in a strong variable. The callee here also keeps the object in a strong global, so the two
// owners are counted exactly; dropping the global does not free the object while the caller holds it,
// and the caller's scope end frees it while the pool is still open.
#include "poly_mobjc.h"
static int deallocs;
static id keep;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; }
@end
__attribute__((noinline)) static Obj *make_kept(void) { Obj *o = [Obj new]; keep = o; return o; }
__attribute__((noinline)) static void drop_kept(void) { keep = nil; }
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    {
        Obj *x = make_kept();
        __unsafe_unretained Obj *u = x;
        value("retainCount: the global and the caller", (i64)retain_count(u));
        drop_kept();
        value("deallocs while the caller holds it", deallocs);
    }
    value("deallocs at the end of the scope", deallocs);
    objc_autoreleasePoolPop(p);
    value("deallocs after the pop", deallocs);
    return finish();
}
