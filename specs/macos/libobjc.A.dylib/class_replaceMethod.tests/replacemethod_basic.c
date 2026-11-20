// poly-mac: libobjc
// class_replaceMethod: adds or replaces; returns the class's own previous IMP, or NULL when it had none of its own.
#include "poly_mobjc.h"

static long v1(id self, SEL _cmd) { (void)self; (void)_cmd; return 1; }
static long v2(id self, SEL _cmd) { (void)self; (void)_cmd; return 2; }
static long v3(id self, SEL _cmd) { (void)self; (void)_cmd; return 3; }

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class c = objc_allocateClassPair(ns, "RmBasic", 0);
    objc_registerClassPair(c);
    SEL s = sel_registerName("val");
    SEL hash = sel_registerName("hash");
    check("a new selector: returns NULL", class_replaceMethod(c, s, (IMP)v1, "q@:") == 0, 0);
    check("it is now there", class_getInstanceMethod(c, s) != 0, 0);
    check("it is the given implementation", method_getImplementation(class_getInstanceMethod(c, s)) == (IMP)v1, 0);
    id o = send0((id)c, "new");
    check("it answers 1", send0l(o, "val") == 1, 0);
    check("replacing returns the previous implementation", class_replaceMethod(c, s, (IMP)v2, "q@:") == (IMP)v1, 0);
    check("the next message uses the new one", send0l(o, "val") == 2, 0);
    check("again", class_replaceMethod(c, s, (IMP)v3, "q@:") == (IMP)v2, 0);
    check("and again the new one is used", send0l(o, "val") == 3, 0);

    /* an inherited method */
    IMP nshash = method_getImplementation(class_getInstanceMethod(ns, hash));
    check("replacing an inherited method returns NULL", class_replaceMethod(c, hash, (IMP)v1, "Q@:") == 0, 0);
    check("the class now has its own", method_getImplementation(class_getInstanceMethod(c, hash)) == (IMP)v1, 0);
    check("the superclass is unchanged", method_getImplementation(class_getInstanceMethod(ns, hash)) == nshash, 0);
    check("an NSObject instance still hashes the old way", send0l(send0((id)ns, "new"), "hash") != 1, 0);
    check("the override is used", send0l(o, "hash") == 1, 0);
    check("replacing again returns the class's own", class_replaceMethod(c, hash, (IMP)v2, "Q@:") == (IMP)v1, 0);

    /* the type string of an existing method is kept; a new method takes the given one */
    class_replaceMethod(c, sel_registerName("typed"), (IMP)v1, "q@:");
    value_str("type after adding", method_getTypeEncoding(class_getInstanceMethod(c, sel_registerName("typed"))));
    class_replaceMethod(c, sel_registerName("typed"), (IMP)v2, "v@:");
    value_str("type after replacing with another string", method_getTypeEncoding(class_getInstanceMethod(c, sel_registerName("typed"))));
    value("the implementation was replaced", method_getImplementation(class_getInstanceMethod(c, sel_registerName("typed"))) == (IMP)v2);
    send0(o, "release");
    return finish();
}
