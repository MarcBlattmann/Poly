// poly-mac: libobjc
// method_getImplementation after class_addMethod / class_replaceMethod, on a class built at run time.
#include "poly_mobjc.h"

static long v1(id self, SEL _cmd) { (void)self; (void)_cmd; return 1; }
static long v2(id self, SEL _cmd) { (void)self; (void)_cmd; return 2; }
static long v3(id self, SEL _cmd) { (void)self; (void)_cmd; return 3; }

int test_main(void) {
    Class a = objc_allocateClassPair(objc_getClass("NSObject"), "MirA", 0);
    SEL s = sel_registerName("dyn");
    class_addMethod(a, s, (IMP)v1, "q@:");
    objc_registerClassPair(a);
    Method m = class_getInstanceMethod(a, s);
    check("the added IMP", method_getImplementation(m) == (IMP)v1, 0);
    class_replaceMethod(a, s, (IMP)v2, "q@:");
    check("the replaced IMP is read through the same Method", method_getImplementation(m) == (IMP)v2, 0);
    Class b = objc_allocateClassPair(a, "MirB", 0);
    objc_registerClassPair(b);
    check("a subclass's Method is the same object and so has the same IMP", method_getImplementation(class_getInstanceMethod(b, s)) == (IMP)v2, 0);
    class_addMethod(b, s, (IMP)v3, "q@:");
    check("a subclass override has its own", method_getImplementation(class_getInstanceMethod(b, s)) == (IMP)v3, 0);
    check("the superclass's is unchanged", method_getImplementation(m) == (IMP)v2, 0);
    /* NSObject methods */
    Class ns = objc_getClass("NSObject");
    IMP h = method_getImplementation(class_getInstanceMethod(ns, sel_registerName("hash")));
    check("an NSObject method has an IMP", h != 0, 0);
    id o = send0((id)ns, "new");
    check("calling NSObject's -hash IMP agrees with a message", ((unsigned long (*)(id, SEL))h)(o, sel_registerName("hash")) == (unsigned long)send0l(o, "hash"), 0);
    check("a class method through the metaclass", method_getImplementation(class_getInstanceMethod(object_getClass((id)ns), sel_registerName("new"))) != 0, 0);
    send0(o, "release");
    return finish();
}
