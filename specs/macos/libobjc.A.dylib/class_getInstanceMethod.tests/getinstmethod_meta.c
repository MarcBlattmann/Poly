// poly-mac: libobjc
// class_getInstanceMethod on a metaclass finds class methods; the root metaclass reaches the root class's instance methods.
#include "poly_mobjc.h"

@interface GmmBase : NSObject
+ (int)classOnly;
- (int)instOnly;
@end
@implementation GmmBase
+ (int)classOnly { return 3; }
- (int)instOnly { return 1; }
@end
@interface GmmLeaf : GmmBase
+ (int)leafClass;
@end
@implementation GmmLeaf
+ (int)leafClass { return 5; }
@end

int test_main(void) {
    Class base = objc_getClass("GmmBase"), leaf = objc_getClass("GmmLeaf"), ns = objc_getClass("NSObject");
    Class bm = object_getClass((id)base), lm = object_getClass((id)leaf), nm = object_getClass((id)ns);
    SEL classOnly = @selector(classOnly), instOnly = @selector(instOnly), leafClass = @selector(leafClass);
    check("class method found on the metaclass", class_getInstanceMethod(bm, classOnly) != 0, 0);
    check("inherited class method found on the subclass's metaclass", class_getInstanceMethod(lm, classOnly) != 0, 0);
    check("same Method as the superclass's metaclass has", class_getInstanceMethod(lm, classOnly) == class_getInstanceMethod(bm, classOnly), 0);
    check("a leaf class method", class_getInstanceMethod(lm, leafClass) != 0, 0);
    check("not on the base metaclass", class_getInstanceMethod(bm, leafClass) == 0, 0);
    check("instance method of the class is absent on its metaclass", class_getInstanceMethod(bm, instOnly) == 0, 0);
    check("class method is absent on the class", class_getInstanceMethod(base, classOnly) == 0, 0);
    check("NSObject +alloc is on NSObject's metaclass", class_getInstanceMethod(nm, sel_registerName("alloc")) != 0, 0);
    check("+alloc is inherited by every metaclass", class_getInstanceMethod(lm, sel_registerName("alloc")) == class_getInstanceMethod(nm, sel_registerName("alloc")), 0);
    check("NSObject's metaclass sees NSObject's instance methods (root class fallback): -finalize", class_getInstanceMethod(nm, sel_registerName("finalize")) == class_getInstanceMethod(ns, sel_registerName("finalize")), 0);
    check("... and so does a subclass's metaclass", class_getInstanceMethod(lm, sel_registerName("finalize")) == class_getInstanceMethod(ns, sel_registerName("finalize")), 0);
    value("root metaclass superclass is NSObject", class_getSuperclass(nm) == ns);
    value("an instance method of a subclass is not on any metaclass", class_getInstanceMethod(lm, instOnly) == 0);
    return finish();
}
