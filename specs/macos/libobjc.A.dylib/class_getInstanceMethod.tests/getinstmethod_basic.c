// poly-mac: libobjc
// class_getInstanceMethod: own and inherited instance methods; NULL when absent.
#include "poly_mobjc.h"

@interface GmBase : NSObject
- (int)baseOnly;
- (int)both;
+ (int)classOnly;
@end
@implementation GmBase
- (int)baseOnly { return 1; }
- (int)both { return 2; }
+ (int)classOnly { return 3; }
@end
@interface GmLeaf : GmBase
- (int)both;
- (int)leafOnly;
@end
@implementation GmLeaf
- (int)both { return 20; }
- (int)leafOnly { return 4; }
@end

int test_main(void) {
    Class base = objc_getClass("GmBase"), leaf = objc_getClass("GmLeaf"), ns = objc_getClass("NSObject");
    SEL baseOnly = @selector(baseOnly), both = @selector(both), leafOnly = @selector(leafOnly), classOnly = @selector(classOnly);
    check("own method of the base", class_getInstanceMethod(base, baseOnly) != 0, 0);
    check("own method of the leaf", class_getInstanceMethod(leaf, leafOnly) != 0, 0);
    check("a method defined by both: leaf", class_getInstanceMethod(leaf, both) != 0, 0);
    check("an inherited method is found", class_getInstanceMethod(leaf, baseOnly) != 0, 0);
    check("a leaf method is not in the base", class_getInstanceMethod(base, leafOnly) == 0, 0);
    check("an absent selector gives NULL", class_getInstanceMethod(leaf, sel_registerName("noSuchMethod")) == 0, 0);
    check("a class method is not an instance method", class_getInstanceMethod(base, classOnly) == 0, 0);
    check("an inherited NSObject method: -hash", class_getInstanceMethod(leaf, @selector(hash)) != 0, 0);
    check("NSObject has no -alloc instance method", class_getInstanceMethod(ns, sel_registerName("alloc")) == 0, 0);
    check("nil class gives NULL", class_getInstanceMethod(Nil, both) == 0, 0);

    Method mb = class_getInstanceMethod(base, both), ml = class_getInstanceMethod(leaf, both);
    check("base and leaf have different Method objects for 'both'", mb != ml, 0);
    check("the base's one has its implementation", ((int (*)(id, SEL))method_getImplementation(mb))(nil, both) == 2, 0);
    check("the leaf's one has its implementation", ((int (*)(id, SEL))method_getImplementation(ml))(nil, both) == 20, 0);
    check("the same call twice gives the same Method", class_getInstanceMethod(leaf, both) == ml, 0);
    check("an inherited method is the superclass's Method", class_getInstanceMethod(leaf, baseOnly) == class_getInstanceMethod(base, baseOnly), 0);
    check("an NSObject method is the same through every subclass", class_getInstanceMethod(leaf, @selector(hash)) == class_getInstanceMethod(ns, @selector(hash)), 0);
    check("method_getName gives the selector back", method_getName(mb) == both, 0);
    value_str("name", sel_getName(method_getName(ml)));
    return finish();
}
