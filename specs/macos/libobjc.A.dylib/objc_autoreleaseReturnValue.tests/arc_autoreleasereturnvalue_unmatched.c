// poly-mac: libobjc
// objc_autoreleaseReturnValue: returns its argument; when the caller does not pair it with
// objc_retainAutoreleasedReturnValue, it autoreleases like objc_autorelease.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
__attribute__((noinline)) static id ar(id o) { return objc_autoreleaseReturnValue(o); }
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    id o = [Obj new];
    id r = ar(o);
    check("returns its argument", r == o, 0);
    value("retainCount", (i64)retain_count(r));
    value("deallocs before pop", deallocs);
    objc_autoreleasePoolPop(p);
    value("deallocs after pop", deallocs);
    check("nil in, nil out", ar(nil) == nil, 0);
    // a second time, with another object
    p = objc_autoreleasePoolPush();
    deallocs = 0;
    o = [Obj new];
    ar(o);
    value("retainCount (second)", (i64)retain_count(o));
    objc_autoreleasePoolPop(p);
    value("deallocs (second)", deallocs);
    return finish();
}
