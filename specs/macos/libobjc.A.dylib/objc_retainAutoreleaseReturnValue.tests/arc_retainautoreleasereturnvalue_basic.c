// poly-mac: libobjc
// objc_retainAutoreleaseReturnValue: retains and autoreleases (as objc_retainAutorelease)
// when its caller does not pair it; returns its argument; accepts nil.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
__attribute__((noinline)) static id rar(id o) { return objc_retainAutoreleaseReturnValue(o); }
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    id o = [Obj new];
    id r = rar(o);
    check("returns its argument", r == o, 0);
    value("retainCount after the call", (i64)retain_count(o));
    objc_autoreleasePoolPop(p);
    value("retainCount after the pop", (i64)retain_count(o));
    value("deallocs after the pop", deallocs);
    objc_release(o);
    value("deallocs after the release", deallocs);
    check("nil in, nil out", rar(nil) == nil, 0);
    return finish();
}
