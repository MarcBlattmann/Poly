// poly-mac: libobjc
// imp_implementationWithBlock: one IMP used for several selectors and classes; self differs per receiver; replace/exchange work.
#include "poly_mobjc.h"

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class a = objc_allocateClassPair(ns, "IbsA", 0), b = objc_allocateClassPair(ns, "IbsB", 0);
    objc_registerClassPair(a); objc_registerClassPair(b);
    __block long calls = 0;
    IMP imp = imp_implementationWithBlock(^long(id self) { calls++; return (long)object_getClass(self) == (long)objc_getClass("IbsA") ? 1 : 2; });
    check("added to class A", class_addMethod(a, sel_registerName("who"), imp, "q@:"), 0);
    check("same IMP added to class B", class_addMethod(b, sel_registerName("who"), imp, "q@:"), 0);
    check("and under another selector", class_addMethod(a, sel_registerName("whoElse"), imp, "q@:"), 0);
    id oa = send0((id)a, "new"), ob = send0((id)b, "new");
    check("A", send0l(oa, "who") == 1, 0);
    check("B", send0l(ob, "who") == 2, 0);
    check("other selector", send0l(oa, "whoElse") == 1, 0);
    value("block calls", calls);

    /* replace and exchange */
    IMP other = imp_implementationWithBlock(^long(id self) { (void)self; return 77; });
    IMP prev = class_replaceMethod(a, sel_registerName("who"), other, "q@:");
    check("class_replaceMethod returns the block IMP", prev == imp, 0);
    check("replacement runs", send0l(oa, "who") == 77, 0);
    method_exchangeImplementations(class_getInstanceMethod(a, sel_registerName("who")), class_getInstanceMethod(a, sel_registerName("whoElse")));
    check("exchange: who is the old block", send0l(oa, "who") == 1, 0);
    check("exchange: whoElse is the replacement", send0l(oa, "whoElse") == 77, 0);
    check("method_getImplementation returns the block IMP", method_getImplementation(class_getInstanceMethod(a, sel_registerName("whoElse"))) == other, 0);
    check("it can be called directly with self and _cmd", ((long (*)(id, SEL))other)(oa, sel_registerName("anything")) == 77, 0);
    /* class methods via the metaclass: self is the class */
    IMP cm = imp_implementationWithBlock(^long(id self) { return (long)class_getName((Class)self)[0]; });
    class_addMethod(object_getClass((id)a), sel_registerName("initial"), cm, "q@:");
    check("a block as a class method sees the class as self", send0l((id)a, "initial") == 'I', 0);
    check("... for a subclass too", (objc_registerClassPair(objc_allocateClassPair(a, "ZSub", 0)), send0l((id)objc_getClass("ZSub"), "initial") == 'Z'), 0);
    object_dispose(oa); object_dispose(ob);
    return finish();
}
