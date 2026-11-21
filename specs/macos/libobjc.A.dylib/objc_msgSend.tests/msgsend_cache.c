// poly-mac: libobjc
// objc_msgSend after the method tables change: the dispatch cache never
// serves a stale method. Methods added, replaced and swapped at run time
// take effect for the very next send, also on selectors already sent.
#include "poly_mobjc.h"

@interface A : NSObject
- (int)v;
- (int)w;
@end
@interface B : A
@end
@interface C : B
- (int)v;
@end
@implementation A
- (int)v { return 1; }
- (int)w { return 10; }
@end
@implementation B
@end
@implementation C
- (int)v { return 3; }
@end

static int imp2(id self, SEL cmd) { (void)self; (void)cmd; return 2; }
static int imp4(id self, SEL cmd) { (void)self; (void)cmd; return 4; }
static int imp5(id self, SEL cmd) { (void)self; (void)cmd; return 5; }

#define V(o) ((int (*)(id, SEL))objc_msgSend)((id)(o), sel_registerName("v"))
#define W(o) ((int (*)(id, SEL))objc_msgSend)((id)(o), sel_registerName("w"))

int test_main(void) {
    A *a = [A new];
    B *b = [B new];
    C *c = [C new];
    Class A_ = objc_getClass("A"), B_ = objc_getClass("B");
    for (int i = 0; i < 4; i++) { V(a); V(b); V(c); }   /* fill the caches */
    check("a", V(a) == 1, V(a));
    check("b inherits", V(b) == 1, V(b));
    check("c overrides", V(c) == 3, V(c));

    /* A method added to the middle class overrides the inherited one for
     * it and for its subclasses that do not define their own. */
    check("class_addMethod on B", class_addMethod(B_, sel_registerName("v"), (IMP)imp2, "i@:"), 0);
    check("b sees the new method at once", V(b) == 2, V(b));
    check("a is untouched", V(a) == 1, V(a));
    check("c keeps its own", V(c) == 3, V(c));

    /* Replacing the implementation on the root class changes the instances
     * that inherit it. */
    class_replaceMethod(A_, sel_registerName("w"), (IMP)imp4, "i@:");
    check("a sees the replaced method", W(a) == 4, W(a));
    check("b inherits the replaced method", W(b) == 4, W(b));
    check("c inherits the replaced method", W(c) == 4, W(c));

    /* method_setImplementation changes the method of the class that owns it. */
    Method mw = class_getInstanceMethod(B_, sel_registerName("w"));
    IMP old = method_setImplementation(mw, (IMP)imp5);
    check("old implementation returned", old == (IMP)imp4, 0);
    check("subclass sees the change", W(c) == 5 && W(a) == 5, W(c));

    /* Swapping two methods swaps what the selectors reach. */
    Method mv = class_getInstanceMethod(A_, sel_registerName("v"));
    Method mw2 = class_getInstanceMethod(A_, sel_registerName("w"));
    method_exchangeImplementations(mv, mw2);
    check("v now runs w's code", V(a) == 5, V(a));
    check("w now runs v's code", W(a) == 1, W(a));
    method_exchangeImplementations(mv, mw2);
    check("swapped back: v", V(a) == 1, V(a));
    check("swapped back: w", W(a) == 5, W(a));

    /* A class method added to the metaclass at run time. */
    check("add a class method", class_addMethod(object_getClass((id)A_), sel_registerName("w"), (IMP)imp4, "i@:"), 0);
    check("+w", W(A_) == 4, W(A_));
    return finish();
}
