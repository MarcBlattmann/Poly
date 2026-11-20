// poly-mac: libobjc
// poly-mac: arc
// objc_retainAutoreleaseReturnValue as ARC-compiled code uses it: a function (not inlined) returns
// one of its arguments. The caller that keeps the result in a strong variable ends with one extra
// count, nothing is queued in the pool, and the object dies when the last variable goes out of scope.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; }
@end
__attribute__((noinline)) static id same(id o) { return o; }
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    {
        Obj *x = [Obj new];
        id y = same(x);
        __unsafe_unretained id u = y;
        value("retainCount with two strong variables", (i64)retain_count(u));
        value("deallocs while both live", deallocs);
    }
    value("deallocs at the end of the scope", deallocs);
    objc_autoreleasePoolPop(p);
    value("deallocs after the pop", deallocs);
    return finish();
}
