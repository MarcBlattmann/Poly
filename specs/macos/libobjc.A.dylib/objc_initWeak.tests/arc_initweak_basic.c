// poly-mac: libobjc
// objc_initWeak: initializes an uninitialized location (any garbage in it is ignored) as a weak
// reference to the object and returns the object; nil gives a nil location and nil result.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id loc = (id)0x1;     // uninitialized storage: garbage
    id r = objc_initWeak(&loc, o);
    check("returns the object", r == o, 0);
    check("the location holds the object", loc == o, 0);
    value("retainCount unchanged", (i64)retain_count(o));
    id l = objc_loadWeakRetained(&loc);
    check("loads it back", l == o, 0);
    objc_release(l);
    id loc2 = (id)0x1;
    id rn = objc_initWeak(&loc2, nil);
    check("nil: returns nil", rn == nil, 0);
    check("nil: the location is nil", loc2 == nil, 0);
    objc_release(o);
    value("deallocs", deallocs);
    check("the location is zeroed when the object dies", loc == nil, 0);
    objc_destroyWeak(&loc);
    objc_destroyWeak(&loc2);
    return finish();
}
