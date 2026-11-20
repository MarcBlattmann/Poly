// poly-mac: libobjc
// imp_implementationWithBlock: an IMP that calls the block with self (and the arguments), without _cmd.
#include "poly_mobjc.h"

typedef struct { long a, b; } Pair;

int test_main(void) {
    Class c = objc_allocateClassPair(objc_getClass("NSObject"), "IbBasic", 0);
    objc_registerClassPair(c);
    long captured = 1000;

    IMP i0 = imp_implementationWithBlock(^long(id self) { (void)self; return 42; });
    check("an IMP is returned", i0 != 0, 0);
    check("installed as a method", class_addMethod(c, sel_registerName("fortyTwo"), i0, "q@:"), 0);
    id o = send0((id)c, "new");
    check("a message calls the block", send0l(o, "fortyTwo") == 42, 0);

    IMP i1 = imp_implementationWithBlock(^id(id self) { return self; });
    class_addMethod(c, sel_registerName("me"), i1, "@@:");
    check("the block receives self", send0(o, "me") == o, 0);

    IMP i2 = imp_implementationWithBlock(^long(id self, long a, long b) { (void)self; return a * 10 + b; });
    class_addMethod(c, sel_registerName("mix:with:"), i2, "q@:qq");
    check("arguments follow self", ((long (*)(id, SEL, long, long))objc_msgSend)(o, sel_registerName("mix:with:"), 4, 7) == 47, 0);

    IMP i3 = imp_implementationWithBlock(^long(id self) { (void)self; return captured; });
    class_addMethod(c, sel_registerName("cap"), i3, "q@:");
    captured = 2000;
    check("a captured variable is copied when the IMP is made", send0l(o, "cap") == 1000, 0);

    IMP i4 = imp_implementationWithBlock(^double(id self, double x) { (void)self; return x * 2.0; });
    class_addMethod(c, sel_registerName("dbl:"), i4, "d@:d");
    check("a double argument and result", ((double (*)(id, SEL, double))objc_msgSend)(o, sel_registerName("dbl:"), 1.25) == 2.5, 0);

    IMP i5 = imp_implementationWithBlock(^Pair(id self, long a) { (void)self; Pair p = {a, a + 1}; return p; });
    class_addMethod(c, sel_registerName("pair:"), i5, "{Pair=qq}@:q");
    Pair p = ((Pair (*)(id, SEL, long))objc_msgSend)(o, sel_registerName("pair:"), 9);
    check("a struct result", p.a == 9 && p.b == 10, 0);

    IMP i6 = imp_implementationWithBlock(^void(id self) { (void)self; });
    class_addMethod(c, sel_registerName("nothing"), i6, "v@:");
    send0(o, "nothing");

    check("every IMP is distinct", i0 != i1 && i1 != i2 && i2 != i3 && i3 != i4, 0);
    IMP j = imp_implementationWithBlock(^long(id self) { (void)self; return 42; });
    check("even for an identical block", j != i0, 0);
    send0(o, "release");
    return finish();
}
