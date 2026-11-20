// poly-mac: libobjc
// class_addMethod: YES for a new method, NO when the class already has it itself, YES when only a superclass has it.
#include "poly_mobjc.h"

static long v1(id self, SEL _cmd) { (void)self; (void)_cmd; return 1; }
static long v2(id self, SEL _cmd) { (void)self; (void)_cmd; return 2; }
static long v3(id self, SEL _cmd) { (void)self; (void)_cmd; return 3; }
static unsigned long my_hash(id self, SEL _cmd) { (void)self; (void)_cmd; return 777; }

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class c = objc_allocateClassPair(ns, "AmBasic", 0);
    SEL one = sel_registerName("one");
    SEL hash = sel_registerName("hash");
    check("a new selector is added", class_addMethod(c, one, (IMP)v1, "q@:"), 0);
    check("the class itself now has it", class_getInstanceMethod(c, one) != 0, 0);
    check("adding it again returns NO", class_addMethod(c, one, (IMP)v2, "q@:") == 0, 0);
    check("a different type string does not matter", class_addMethod(c, one, (IMP)v2, "v@:") == 0, 0);
    check("the first implementation is kept", method_getImplementation(class_getInstanceMethod(c, one)) == (IMP)v1, 0);
    check("the first type string is kept", str_eq(method_getTypeEncoding(class_getInstanceMethod(c, one)), "q@:"), 0);

    check("NSObject has hash itself", class_getInstanceMethod(ns, hash) != 0, 0);
    IMP nshash = method_getImplementation(class_getInstanceMethod(ns, hash));
    check("a method only the superclass has can be added (override)", class_addMethod(c, hash, (IMP)my_hash, "Q@:"), 0);
    check("the superclass keeps its own", method_getImplementation(class_getInstanceMethod(ns, hash)) == nshash, 0);
    check("the class has its own now", method_getImplementation(class_getInstanceMethod(c, hash)) == (IMP)my_hash, 0);
    objc_registerClassPair(c);

    id o = send0((id)c, "new");
    check("the override is used", ((unsigned long (*)(id, SEL))objc_msgSend)(o, hash) == 777, 0);
    check("the new method is used", send0l(o, "one") == 1, 0);
    send0(o, "release");

    /* after registration */
    check("adding a new selector after registration", class_addMethod(c, sel_registerName("two"), (IMP)v2, "q@:"), 0);
    check("adding an existing one after registration is NO", class_addMethod(c, sel_registerName("two"), (IMP)v3, "q@:") == 0, 0);
    o = send0((id)c, "new");
    check("the added method answers", send0l(o, "two") == 2, 0);
    send0(o, "release");

    /* a subclass is unaffected by its superclass having a method added later only for selectors it overrides itself */
    Class d = objc_allocateClassPair(c, "AmBasicSub", 0);
    class_addMethod(d, sel_registerName("three"), (IMP)v3, "q@:");
    objc_registerClassPair(d);
    check("adding to the superclass later does not stop a subclass method", class_addMethod(c, sel_registerName("three"), (IMP)v1, "q@:"), 0);
    o = send0((id)d, "new");
    check("the subclass keeps its own implementation", send0l(o, "three") == 3, 0);
    check("an inherited added method is seen", send0l(o, "one") == 1, 0);
    send0(o, "release");
    return finish();
}
