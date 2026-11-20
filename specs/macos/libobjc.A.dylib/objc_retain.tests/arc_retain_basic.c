// poly-mac: libobjc
// objc_retain: returns its argument, adds one to the retain count, nil is accepted.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    check("new object", o != 0, 0);
    value("retainCount of a new object", (i64)retain_count(o));
    id r = objc_retain(o);
    check("returns its argument", r == o, 0);
    value("retainCount after one retain", (i64)retain_count(o));
    objc_retain(o);
    objc_retain(o);
    value("retainCount after three retains", (i64)retain_count(o));
    objc_release(o);
    value("retainCount after one release", (i64)retain_count(o));
    value("deallocs so far", deallocs);
    objc_release(o);
    objc_release(o);
    value("deallocs before the last release", deallocs);
    objc_release(o);
    value("deallocs after the last release", deallocs);
    check("nil in, nil out", objc_retain(nil) == nil, 0);
    check("nil again", objc_retain(0) == 0, 0);
    return finish();
}
