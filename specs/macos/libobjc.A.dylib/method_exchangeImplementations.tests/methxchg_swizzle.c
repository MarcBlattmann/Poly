// poly-mac: libobjc
// method_exchangeImplementations used as swizzling: the replacement calls "itself" to reach the original; types stay.
#include "poly_mobjc.h"

static long orig_calls, swz_calls;
static IMP swz_original;

static long target_orig(id self, SEL _cmd) { (void)self; (void)_cmd; orig_calls++; return 10; }
static long target_swz(id self, SEL _cmd) {
    (void)_cmd;
    swz_calls++;
    /* after the exchange, the selector "swz" holds the original IMP */
    return ((long (*)(id, SEL))objc_msgSend)(self, sel_registerName("swz")) + 1;
}
static long ret5(id self, SEL _cmd) { (void)self; (void)_cmd; return 5; }
static double retd(id self, SEL _cmd) { (void)self; (void)_cmd; return 2.5; }

int test_main(void) {
    Class c = objc_allocateClassPair(objc_getClass("NSObject"), "MxSwz", 0);
    class_addMethod(c, sel_registerName("target"), (IMP)target_orig, "q@:");
    class_addMethod(c, sel_registerName("swz"), (IMP)target_swz, "q@:");
    class_addMethod(c, sel_registerName("a"), (IMP)ret5, "q@:");
    class_addMethod(c, sel_registerName("b"), (IMP)retd, "d@:");
    objc_registerClassPair(c);
    id o = send0((id)c, "new");
    swz_original = method_getImplementation(class_getInstanceMethod(c, sel_registerName("target")));
    check("before: target is the original", send0l(o, "target") == 10 && orig_calls == 1 && swz_calls == 0, orig_calls);
    method_exchangeImplementations(class_getInstanceMethod(c, sel_registerName("target")), class_getInstanceMethod(c, sel_registerName("swz")));
    long r = send0l(o, "target");
    check("after: target runs the replacement, which reaches the original through swz", r == 11, r);
    value("original calls", orig_calls);
    value("replacement calls", swz_calls);
    check("swz now holds the original IMP", method_getImplementation(class_getInstanceMethod(c, sel_registerName("swz"))) == swz_original, 0);

    /* type encodings stay with their Methods */
    Method ma = class_getInstanceMethod(c, sel_registerName("a")), mb = class_getInstanceMethod(c, sel_registerName("b"));
    method_exchangeImplementations(ma, mb);
    value_str("a's encoding after the exchange", method_getTypeEncoding(ma));
    value_str("b's encoding after the exchange", method_getTypeEncoding(mb));
    check("encodings were not swapped", str_eq(method_getTypeEncoding(ma), "q@:") && str_eq(method_getTypeEncoding(mb), "d@:"), 0);
    send0(o, "release");
    return finish();
}
