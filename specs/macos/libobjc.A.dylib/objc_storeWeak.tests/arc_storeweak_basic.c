// poly-mac: libobjc
// objc_storeWeak: registers a location, returns the object stored, does not retain it, zeroes
// the location when the object is deallocated; nil clears the registration.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id loc = 0, loc2 = 0;
    id r = objc_storeWeak(&loc, o);
    check("returns the object", r == o, 0);
    check("the location holds the object", loc == o, 0);
    value("retainCount unchanged by the store", (i64)retain_count(o));
    objc_storeWeak(&loc2, o);
    objc_release(o);
    value("deallocs", deallocs);
    check("the first location was zeroed", loc == nil, 0);
    check("the second location was zeroed", loc2 == nil, 0);
    // clearing
    id p = [Obj new]; id loc3 = 0;
    objc_storeWeak(&loc3, p);
    id rn = objc_storeWeak(&loc3, nil);
    check("storing nil returns nil", rn == nil, 0);
    check("the location is nil", loc3 == nil, 0);
    objc_release(p);
    check("the cleared location stays nil", loc3 == nil, 0);
    check("storing nil in a nil location", objc_storeWeak(&loc3, nil) == nil, 0);
    return finish();
}
