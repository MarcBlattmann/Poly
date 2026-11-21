// poly-mac: libobjc
#include "poly_mobjc.h"
static int aw_n, plain_n;
@interface W : NSObject
@end
@implementation W
+ (id)allocWithZone:(struct _NSZone *)z { aw_n++; return [super allocWithZone:z]; }
@end
@interface Pl : NSObject
@end
@implementation Pl
@end
int test_main(void) {
    id a = objc_alloc(objc_getClass("W"));
    value("+allocWithZone: calls for W", aw_n);
    id b = objc_allocWithZone(objc_getClass("W"));
    value("after objc_allocWithZone", aw_n);
    (void)a; (void)b; (void)plain_n;
    check("objc_allocWithZone made an instance", b != 0, 0);
    // a class made at run time
    Class r = objc_allocateClassPair(objc_getClass("NSObject"), "RtCls", 0);
    objc_registerClassPair(r);
    id o = objc_alloc(r);
    value_str("runtime class", cname(o));
    return finish();
}
