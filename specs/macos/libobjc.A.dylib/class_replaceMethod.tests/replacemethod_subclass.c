// poly-mac: libobjc
// class_replaceMethod: effects on subclasses, class methods and a method of a class that was already used.
#include "poly_mobjc.h"

static long v1(id self, SEL _cmd) { (void)self; (void)_cmd; return 1; }
static long v2(id self, SEL _cmd) { (void)self; (void)_cmd; return 2; }
static long v9(id self, SEL _cmd) { (void)self; (void)_cmd; return 9; }

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class a = objc_allocateClassPair(ns, "RmSubA", 0);
    class_addMethod(a, sel_registerName("val"), (IMP)v1, "q@:");
    objc_registerClassPair(a);
    Class b = objc_allocateClassPair(a, "RmSubB", 0);
    Class b2 = (objc_registerClassPair(b), objc_allocateClassPair(a, "RmSubC", 0));
    class_addMethod(b2, sel_registerName("val"), (IMP)v9, "q@:");
    objc_registerClassPair(b2);
    id oa = send0((id)a, "new"), ob = send0((id)b, "new"), oc = send0((id)b2, "new");
    for (int i = 0; i < 3; i++) { (void)send0l(oa, "val"); (void)send0l(ob, "val"); (void)send0l(oc, "val"); }
    SEL val = sel_registerName("val");
    check("replacing in the superclass returns its old IMP", class_replaceMethod(a, val, (IMP)v2, "q@:") == (IMP)v1, 0);
    check("the superclass uses the new", send0l(oa, "val") == 2, 0);
    check("a subclass without its own sees it", send0l(ob, "val") == 2, 0);
    check("a subclass with its own keeps it", send0l(oc, "val") == 9, 0);
    check("replacing in the subclass returns NULL (inherited only)", class_replaceMethod(b, val, (IMP)v1, "q@:") == 0, 0);
    check("subclass now uses its own", send0l(ob, "val") == 1, 0);
    check("superclass keeps the replaced", send0l(oa, "val") == 2, 0);

    /* class methods through the metaclass */
    Class meta = object_getClass((id)a);
    SEL cm = sel_registerName("cval");
    check("class method new: NULL", class_replaceMethod(meta, cm, (IMP)v1, "q@:") == 0, 0);
    check("class method works", send0l((id)a, "cval") == 1, 0);
    check("class method replaced: returns old", class_replaceMethod(meta, cm, (IMP)v2, "q@:") == (IMP)v1, 0);
    check("class method updated", send0l((id)a, "cval") == 2, 0);
    check("subclass inherits the class method", send0l((id)b, "cval") == 2, 0);
    object_dispose(oa); object_dispose(ob); object_dispose(oc);
    return finish();
}
