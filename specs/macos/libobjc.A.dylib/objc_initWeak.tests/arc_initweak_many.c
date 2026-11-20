// poly-mac: libobjc
// objc_initWeak: several locations; initWeak and storeWeak mixed; zeroing at dealloc for all of them.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id a = (id)0x1, b = (id)0x1, c = 0;
    objc_initWeak(&a, o);
    objc_initWeak(&b, o);
    objc_storeWeak(&c, o);
    value("a", a == o); value("b", b == o); value("c", c == o);
    objc_destroyWeak(&b);
    objc_release(o);
    value("deallocs", deallocs);
    check("a zeroed", a == nil, 0);
    check("c zeroed", c == nil, 0);
    // initWeak on a class object
    id k = (id)objc_getClass("NSObject");
    id lk = (id)0x1;
    check("class object", objc_initWeak(&lk, k) == k, 0);
    check("class object location", lk == k, 0);
    objc_destroyWeak(&lk);
    objc_destroyWeak(&a);
    objc_destroyWeak(&c);
    return finish();
}
