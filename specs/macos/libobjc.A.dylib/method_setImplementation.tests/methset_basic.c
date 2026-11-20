// poly-mac: libobjc
// method_setImplementation: returns the old IMP; messages sent afterwards (even to warmed-up call sites) use the new one.
#include "poly_mobjc.h"

static long v1(id self, SEL _cmd) { (void)self; (void)_cmd; return 1; }
static long v2(id self, SEL _cmd) { (void)self; (void)_cmd; return 2; }
static long v3(id self, SEL _cmd) { (void)self; (void)_cmd; return 3; }

@interface MsBase : NSObject
- (long)val;
@end
@implementation MsBase
- (long)val { return 100; }
@end
@interface MsLeaf : MsBase
@end
@implementation MsLeaf
@end

int test_main(void) {
    Class a = objc_allocateClassPair(objc_getClass("NSObject"), "MsA", 0);
    SEL s = sel_registerName("val");
    class_addMethod(a, s, (IMP)v1, "q@:");
    objc_registerClassPair(a);
    id o = send0((id)a, "new");
    for (int i = 0; i < 10; i++) (void)send0l(o, "val");
    Method m = class_getInstanceMethod(a, s);
    check("returns the previous IMP", method_setImplementation(m, (IMP)v2) == (IMP)v1, 0);
    check("the Method has the new IMP", method_getImplementation(m) == (IMP)v2, 0);
    check("the next message (warm cache) calls it", send0l(o, "val") == 2, 0);
    check("again", method_setImplementation(m, (IMP)v3) == (IMP)v2, 0);
    check("and calls the newest", send0l(o, "val") == 3, 0);
    check("setting the same IMP returns it", method_setImplementation(m, (IMP)v3) == (IMP)v3, 0);
    check("the type encoding is kept", str_eq(method_getTypeEncoding(m), "q@:"), 0);
    check("the selector is kept", method_getName(m) == s, 0);
    check("class_getMethodImplementation sees it", class_getMethodImplementation(a, s) == (IMP)v3, 0);

    /* a method of a compiled class, with a subclass inheriting it */
    Class base = objc_getClass("MsBase"), leaf = objc_getClass("MsLeaf");
    id ob = send0((id)base, "new"), ol = send0((id)leaf, "new");
    check("before: base", send0l(ob, "val") == 100, 0);
    check("before: leaf", send0l(ol, "val") == 100, 0);
    Method mb = class_getInstanceMethod(base, s);
    IMP old = method_setImplementation(mb, (IMP)v1);
    check("the old IMP can be called", ((long (*)(id, SEL))old)(ob, s) == 100, 0);
    check("base uses the new one", send0l(ob, "val") == 1, 0);
    check("a subclass that inherits the method uses it too", send0l(ol, "val") == 1, 0);
    method_setImplementation(mb, old);
    check("restoring the old IMP restores the behaviour", send0l(ob, "val") == 100 && send0l(ol, "val") == 100, 0);
    object_dispose(o); object_dispose(ob); object_dispose(ol);
    return finish();
}
