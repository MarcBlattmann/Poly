// poly-mac: libobjc
// objc_retainAutoreleasedReturnValue: when its caller is not the receiver of a paired
// objc_autoreleaseReturnValue it retains like objc_retain, returns its argument, accepts nil.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id r = objc_retainAutoreleasedReturnValue(o);
    check("returns its argument", r == o, 0);
    value("retainCount after the call", (i64)retain_count(o));
    objc_retainAutoreleasedReturnValue(o);
    value("retainCount after two calls", (i64)retain_count(o));
    objc_release(o); objc_release(o);
    value("deallocs with one reference left", deallocs);
    objc_release(o);
    value("deallocs", deallocs);
    check("nil in, nil out", objc_retainAutoreleasedReturnValue(nil) == nil, 0);
    return finish();
}
