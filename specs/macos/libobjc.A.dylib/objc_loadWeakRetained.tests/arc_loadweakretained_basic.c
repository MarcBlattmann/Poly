// poly-mac: libobjc
// objc_loadWeakRetained: returns the object with one added count (the caller owns it),
// or nil when the location is nil or the object is gone; the location is not changed.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id loc = 0;
    check("an empty location loads nil", objc_loadWeakRetained(&loc) == nil, 0);
    objc_storeWeak(&loc, o);
    value("retainCount before the load", (i64)retain_count(o));
    id l = objc_loadWeakRetained(&loc);
    check("returns the object", l == o, 0);
    value("retainCount after the load", (i64)retain_count(o));
    check("the location is unchanged", loc == o, 0);
    objc_release(l);
    value("retainCount after releasing the loaded reference", (i64)retain_count(o));
    id l2 = objc_loadWeakRetained(&loc);
    id l3 = objc_loadWeakRetained(&loc);
    value("retainCount after two loads", (i64)retain_count(o));
    objc_release(l2); objc_release(l3);
    // the loaded reference keeps the object alive
    id keep = objc_loadWeakRetained(&loc);
    objc_release(o);
    value("deallocs while a loaded reference is held", deallocs);
    check("still loadable", objc_loadWeakRetained(&loc) == keep, 0);
    objc_release(keep);   // the extra from the previous load
    objc_release(keep);
    value("deallocs after the last release", deallocs);
    check("nil after the object is gone", objc_loadWeakRetained(&loc) == nil, 0);
    check("the location was zeroed", loc == nil, 0);
    return finish();
}
