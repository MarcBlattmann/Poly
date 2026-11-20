// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static int deallocs;
@interface Ex : NSObject
@end
@implementation Ex
- (void)dealloc { deallocs++; [super dealloc]; }
@end
@interface ExB : Ex
@end
@implementation ExB
@end
static void thrower(id e) { objc_exception_throw(e); }
int test_main(void) {
    id e = objc_alloc_init(objc_getClass("Ex"));
    int step = 0;
    @try {
        @try { thrower(e); }
        @catch (id x) { step = 1; @throw; }
    } @catch (id y) { check("@throw with no operand rethrows the caught object", y == e, 0); step = 2; value("rc in the final handler", (i64)retain_count(y)); }
    value("steps", step);
    value("rc afterwards", (i64)retain_count(e));
    // rethrow through two handlers
    step = 0;
    @try {
        @try {
            @try { thrower(e); }
            @catch (Ex *a) { step++; @throw; }
        } @catch (Ex *b) { step++; @throw; }
    } @catch (id c) { step++; check("same object after two rethrows", c == e, 0); value("rc after two rethrows, in the last handler", (i64)retain_count(c)); }
    value("handlers run", step);
    value("rc afterwards", (i64)retain_count(e));
    // throwing a different exception from a handler: the first is finished with
    id e2 = objc_alloc_init(objc_getClass("ExB"));
    @try {
        @try { thrower(e); }
        @catch (id x) { thrower(e2); }
    } @catch (id y) { check("the new one arrives", y == e2, 0); value("rc of the first once the handler was left by a new throw", (i64)retain_count(e)); value("rc of the second", (i64)retain_count(e2)); }
    value("rc of the second after", (i64)retain_count(e2));
    // rethrown with a changed type clause matching on the second try
    @try { @try { thrower(e2); } @catch (ExB *x) { @throw; } } @catch (Ex *y) { check("a superclass clause takes the rethrown subclass instance", y == e2, 0); }
    rel(e); rel(e2);
    value("deallocs", deallocs);
    return finish();
}
