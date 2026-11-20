// poly-mac: libobjc
// A class under construction takes methods and variables until it is registered; afterwards ivars are refused.
#include "poly_mobjc.h"

static long seven(id self, SEL _cmd) { (void)self; (void)_cmd; return 7; }

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class c = objc_allocateClassPair(ns, "ApBuild", 0);
    check("allocated", c != Nil, 0);
    value("instance size before registration is that of the superclass", class_getInstanceSize(c) == class_getInstanceSize(ns));
    check("method added before registration", class_addMethod(c, sel_registerName("seven"), (IMP)seven, "q@:"), 0);
    check("ivar added before registration", class_addIvar(c, "_a", 8, 3, "q"), 0);
    check("second ivar", class_addIvar(c, "_b", 4, 2, "i"), 0);
    check("an ivar with a taken name is refused", class_addIvar(c, "_a", 8, 3, "q") == 0, 0);
    objc_registerClassPair(c);
    value("instance size grew by 16 (8 + 4, padded)", class_getInstanceSize(c) - class_getInstanceSize(ns));
    check("an ivar cannot be added after registration", class_addIvar(c, "_c", 8, 3, "q") == 0, 0);
    check("a method can be added after registration", class_addMethod(c, sel_registerName("eight"), (IMP)seven, "q@:"), 0);
    id o = send0((id)c, "new");
    check("instance made", o != nil, 0);
    value("seven", send0l(o, "seven"));
    value("eight", send0l(o, "eight"));
    Ivar iv = class_getInstanceVariable(c, "_a");
    check("ivar found", iv != 0, 0);
    value("ivar _a offset minus NSObject size", ivar_getOffset(iv) - class_getInstanceSize(ns));
    value_str("ivar _a type", ivar_getTypeEncoding(iv));
    send0(o, "release");
    /* the class pair is independent of a second, same-superclass pair */
    Class d = objc_allocateClassPair(ns, "ApBuild2", 0);
    objc_registerClassPair(d);
    check("methods are per class", class_respondsToSelector(d, sel_registerName("seven")) == 0, 0);
    return finish();
}
