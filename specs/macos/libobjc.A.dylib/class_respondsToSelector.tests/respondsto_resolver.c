// poly-mac: libobjc
// class_respondsToSelector consults +resolveInstanceMethod: for a selector the class lacks; what else does.
#include "poly_mobjc.h"

static int resolver_calls;
static int last_was_class;
static long resolved(id self, SEL _cmd) { (void)self; (void)_cmd; return 11; }
static BOOL resolve(id self, SEL _cmd, SEL sel) {
    (void)_cmd;
    resolver_calls++;
    last_was_class = class_isMetaClass(object_getClass(self));
    if (sel == sel_registerName("lazy")) return class_addMethod((Class)self, sel, (IMP)resolved, "q@:");
    return NO;
}
static Class fresh(const char *name) {
    Class c = objc_allocateClassPair(objc_getClass("NSObject"), name, 0);
    class_addMethod(object_getClass((id)c), sel_registerName("resolveInstanceMethod:"), (IMP)resolve, "B@::");
    objc_registerClassPair(c);
    resolver_calls = 0;
    return c;
}

int test_main(void) {
    SEL lazy = sel_registerName("lazy"), nope = sel_registerName("nope");
    BOOL (*rts)(id, SEL, SEL) = (BOOL (*)(id, SEL, SEL))objc_msgSend;
    Class c;

    c = fresh("RtRes1");
    int r = class_respondsToSelector(c, lazy);
    value("respondsToSelector(lazy): result", r);
    value("respondsToSelector(lazy): resolver calls", resolver_calls);
    check("the resolver got the class as receiver", last_was_class, 0);
    check("the resolved method now exists in the class", class_getInstanceMethod(c, lazy) != 0, 0);
    value("respondsToSelector(lazy) again: resolver calls so far", (class_respondsToSelector(c, lazy), resolver_calls));

    c = fresh("RtRes2");
    value("respondsToSelector(nope): result", class_respondsToSelector(c, nope));
    value("respondsToSelector(nope): resolver calls", resolver_calls);
    class_respondsToSelector(c, nope);
    value("respondsToSelector(nope) twice: resolver calls", resolver_calls);

    c = fresh("RtRes3");
    value("class_getInstanceMethod(lazy) result non-NULL", class_getInstanceMethod(c, lazy) != 0);
    value("class_getInstanceMethod(lazy): resolver calls", resolver_calls);

    c = fresh("RtRes3b");
    value("class_getInstanceMethod(nope) result non-NULL", class_getInstanceMethod(c, nope) != 0);
    value("class_getInstanceMethod(nope): resolver calls", resolver_calls);
    value("then class_respondsToSelector(nope) adds calls", (class_respondsToSelector(c, nope), resolver_calls));

    c = fresh("RtRes4");
    value("class_getMethodImplementation(lazy): resolver calls", (class_getMethodImplementation(c, lazy), resolver_calls));
    value("... and then the method exists", class_getInstanceMethod(c, lazy) != 0);

    c = fresh("RtRes5");
    id o = send0((id)c, "new");
    value("-respondsToSelector:(lazy) result", rts(o, sel_registerName("respondsToSelector:"), lazy));
    value("-respondsToSelector:(lazy) resolver calls", resolver_calls);

    c = fresh("RtRes6");
    value("+instancesRespondToSelector:(lazy) result", rts((id)c, sel_registerName("instancesRespondToSelector:"), lazy));
    value("+instancesRespondToSelector:(lazy) resolver calls", resolver_calls);

    c = fresh("RtRes7");
    o = send0((id)c, "new");
    check("a message reaches the resolver and the added method", send0l(o, "lazy") == 11, 0);
    value("message: resolver calls", resolver_calls);
    check("a second message does not call it again", send0l(o, "lazy") == 11 && resolver_calls == 1, resolver_calls);
    send0(o, "release");
    return finish();
}
