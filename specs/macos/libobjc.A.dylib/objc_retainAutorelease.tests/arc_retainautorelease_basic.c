// poly-mac: libobjc
// objc_retainAutorelease: retains and autoreleases; returns its argument; nil is accepted.
#include "poly_mobjc.h"
static int deallocs, c_retain, c_auto;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
- (id)retain { c_retain++; return [super retain]; }
- (id)autorelease { c_auto++; return [super autorelease]; }
@end
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    id o = [Obj new];
    id r = objc_retainAutorelease(o);
    check("returns its argument", r == o, 0);
    value("retainCount after objc_retainAutorelease", (i64)retain_count(o));
    value("-retain calls", c_retain);
    value("-autorelease calls", c_auto);
    objc_autoreleasePoolPop(p);
    value("retainCount after the pop", (i64)retain_count(o));
    value("deallocs after the pop", deallocs);
    objc_release(o);
    value("deallocs after the release", deallocs);
    check("nil in, nil out", objc_retainAutorelease(nil) == nil, 0);
    return finish();
}
