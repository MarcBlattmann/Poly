// poly-mac: libobjc
#include "poly_mobjc.h"
static int seen_sub_init;
@interface Base : NSObject
@end
@implementation Base
- (id)init { self = [super init]; return self; }
@end
@interface Sub : Base
@end
@implementation Sub
- (id)init { seen_sub_init++; return [super init]; }
@end
@interface InitOnClass : NSObject
@end
@implementation InitOnClass
+ (id)init { return nil; }
@end
int test_main(void) {
    id s = objc_alloc_init(objc_getClass("Sub"));
    check("object made", s != 0, 0);
    value_str("subclass", cname(s));
    value("Sub -init calls", seen_sub_init);
    // a class made at run time, with the superclass' -init
    Class r = objc_allocateClassPair(objc_getClass("Sub"), "RtSub", 0);
    objc_registerClassPair(r);
    id t = objc_alloc_init(r);
    value_str("run-time subclass", cname(t));
    value("Sub -init calls after run-time subclass", seen_sub_init);
    // a class method called init does not matter
    id u = objc_alloc_init(objc_getClass("InitOnClass"));
    check("class with +init still gets an instance", u != 0, 0);
    value_str("class with +init", cname(u));
    return finish();
}
