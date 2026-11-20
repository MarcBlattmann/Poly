// poly-mac: libobjc
// method_exchangeImplementations: swaps two methods' IMPs; calling it twice undoes it; types stay with their methods.
#include "poly_mobjc.h"

@interface MxObj : NSObject
- (int)one;
- (int)two;
@end
@implementation MxObj
- (int)one { return 1; }
- (int)two { return 2; }
@end

int test_main(void) {
    Class c = objc_getClass("MxObj");
    SEL one = @selector(one), two = @selector(two);
    id o = send0((id)c, "new");
    check("before: one", send0l(o, "one") == 1, 0);
    check("before: two", send0l(o, "two") == 2, 0);
    Method m1 = class_getInstanceMethod(c, one), m2 = class_getInstanceMethod(c, two);
    IMP i1 = method_getImplementation(m1), i2 = method_getImplementation(m2);
    method_exchangeImplementations(m1, m2);
    check("the Methods keep their selectors", method_getName(m1) == one && method_getName(m2) == two, 0);
    check("the IMPs are swapped", method_getImplementation(m1) == i2 && method_getImplementation(m2) == i1, 0);
    check("messages follow: one now answers 2", send0l(o, "one") == 2, 0);
    check("messages follow: two now answers 1", send0l(o, "two") == 1, 0);
    check("class_getMethodImplementation follows", class_getMethodImplementation(c, one) == i2, 0);
    method_exchangeImplementations(m1, m2);
    check("exchanging again restores one", send0l(o, "one") == 1, 0);
    check("exchanging again restores two", send0l(o, "two") == 2, 0);
    method_exchangeImplementations(m1, m1);
    check("exchanging a Method with itself changes nothing", send0l(o, "one") == 1 && method_getImplementation(m1) == i1, 0);
    check("the same Method objects are still found", class_getInstanceMethod(c, one) == m1 && class_getInstanceMethod(c, two) == m2, 0);
    send0(o, "release");
    return finish();
}
