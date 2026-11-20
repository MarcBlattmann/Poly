// poly-mac: libobjc
// objc_destroyWeak: legal on a location registered by initWeak or storeWeak, on one that is
// nil and on one whose object died; the location may be initialised again afterwards. (What the
// storage holds after the call is unspecified and not looked at.)
#include "poly_mobjc.h"
@interface Obj : NSObject @end
@implementation Obj
@end
int test_main(void) {
    id o = [Obj new];
    id loc = 0;
    objc_initWeak(&loc, o);
    objc_destroyWeak(&loc);
    objc_initWeak(&loc, o);
    check("initialised again after destroyWeak", loc == o, 0);
    id l = objc_loadWeakRetained(&loc);
    check("loads the object", l == o, 0);
    objc_release(l);
    objc_destroyWeak(&loc);
    // destroy after the object died
    id p = [Obj new]; id l2 = 0;
    objc_initWeak(&l2, p);
    objc_release(p);
    check("zeroed by the death", l2 == nil, 0);
    objc_destroyWeak(&l2);
    value("destroy after the death survived", 1);
    // a storeWeak-registered location
    id q = [Obj new]; id l3 = 0;
    objc_storeWeak(&l3, q);
    objc_destroyWeak(&l3);
    objc_initWeak(&l3, q);
    check("registered again", l3 == q, 0);
    objc_destroyWeak(&l3);
    // a nil location
    id l4 = 0;
    objc_initWeak(&l4, nil);
    objc_destroyWeak(&l4);
    value("destroy of a nil location survived", 1);
    objc_release(o); objc_release(q);
    return finish();
}
