// poly-mac: libobjc
// method_getTypeEncoding: the string given when a method was added comes back unchanged; compiled NSObject methods.
#include "poly_mobjc.h"

static long dummy(id self, SEL _cmd) { (void)self; (void)_cmd; return 0; }
static const char *volatile NP = 0;

int test_main(void) {
    Class c = objc_allocateClassPair(objc_getClass("NSObject"), "MeeRuntime", 0);
    const char *encs[] = {"v@:", "q@:q", "@@:@@", "{S=dd}@:{S=dd}", "v32@0:8@16q24", "i", "  ", "^{_NSZone=}", "b4i12"};
    for (unsigned i = 0; i < sizeof encs / sizeof *encs; i++) {
        char name[16] = "m0";
        name[1] = '0' + i;
        class_addMethod(c, sel_registerName(name), (IMP)dummy, encs[i]);
    }
    class_addMethod(c, sel_registerName("nullenc"), (IMP)dummy, NP);
    objc_registerClassPair(c);
    for (unsigned i = 0; i < sizeof encs / sizeof *encs; i++) {
        char name[16] = "m0";
        name[1] = '0' + i;
        const char *e = method_getTypeEncoding(class_getInstanceMethod(c, sel_registerName(name)));
        check(name, e != 0 && str_eq(e, encs[i]), 0);
        value_str(name, e ? e : "(NULL)");
    }
    const char *e = method_getTypeEncoding(class_getInstanceMethod(c, sel_registerName("nullenc")));
    check("a NULL type string reads back as a non-NULL empty string", e != 0 && e[0] == 0, 0);
    /* same string pointer on each call */
    Method m = class_getInstanceMethod(c, sel_registerName("m0"));
    check("pointer stable", method_getTypeEncoding(m) == method_getTypeEncoding(m), 0);
    /* an override keeps the string of the method it was added with, not the superclass's */
    Class ns = objc_getClass("NSObject");
    class_addMethod(c, sel_registerName("hash"), (IMP)dummy, "q@:");
    value_str("NSObject -hash", method_getTypeEncoding(class_getInstanceMethod(ns, sel_registerName("hash"))));
    value_str("override -hash", method_getTypeEncoding(class_getInstanceMethod(c, sel_registerName("hash"))));
    check("two Methods, two strings", method_getTypeEncoding(class_getInstanceMethod(ns, sel_registerName("hash"))) != method_getTypeEncoding(class_getInstanceMethod(c, sel_registerName("hash"))), 0);
    return finish();
}
