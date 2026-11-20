// poly-mac: libobjc
// class_getMethodImplementation follows methods added, replaced and exchanged at run time.
#include "poly_mobjc.h"

static long v1(id self, SEL _cmd) { (void)self; (void)_cmd; return 1; }
static long v2(id self, SEL _cmd) { (void)self; (void)_cmd; return 2; }
static long v3(id self, SEL _cmd) { (void)self; (void)_cmd; return 3; }

int test_main(void) {
    Class a = objc_allocateClassPair(objc_getClass("NSObject"), "GirA", 0);
    objc_registerClassPair(a);
    Class b = objc_allocateClassPair(a, "GirB", 0);
    objc_registerClassPair(b);
    SEL s = sel_registerName("dyn");
    IMP missing = class_getMethodImplementation(b, s);
    check("absent: a forwarding IMP", missing != 0, 0);
    class_addMethod(a, s, (IMP)v1, "q@:");
    check("after adding to the superclass: its IMP", class_getMethodImplementation(a, s) == (IMP)v1, 0);
    check("the subclass sees it (cache is not stale)", class_getMethodImplementation(b, s) == (IMP)v1, 0);
    check("not the forwarding IMP any more", class_getMethodImplementation(b, s) != missing, 0);
    class_addMethod(b, s, (IMP)v2, "q@:");
    check("subclass override", class_getMethodImplementation(b, s) == (IMP)v2, 0);
    check("superclass unchanged", class_getMethodImplementation(a, s) == (IMP)v1, 0);
    class_replaceMethod(b, s, (IMP)v3, "q@:");
    check("after replacement", class_getMethodImplementation(b, s) == (IMP)v3, 0);
    method_setImplementation(class_getInstanceMethod(a, s), (IMP)v3);
    check("after method_setImplementation", class_getMethodImplementation(a, s) == (IMP)v3, 0);
    SEL t = sel_registerName("dyn2");
    class_addMethod(a, t, (IMP)v1, "q@:");
    method_exchangeImplementations(class_getInstanceMethod(a, s), class_getInstanceMethod(a, t));
    check("after exchange (s)", class_getMethodImplementation(a, s) == (IMP)v1, 0);
    check("after exchange (t)", class_getMethodImplementation(a, t) == (IMP)v3, 0);
    id o = send0((id)a, "new");
    check("what is returned is what a message calls", ((long (*)(id, SEL))class_getMethodImplementation(a, s))(o, s) == send0l(o, "dyn"), 0);
    send0(o, "release");
    return finish();
}
