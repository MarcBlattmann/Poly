// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
@interface Ex : NSObject { @public int code; }
@end
@implementation Ex
@end
@interface ExSub : Ex
@end
@implementation ExSub
@end
@interface Other : NSObject
@end
@implementation Other
@end
static void thrower(id e) { objc_exception_throw(e); }
// a C frame between the throw and the handler
static void (*volatile thrower_p)(id) = thrower;
static int through_c(id e, int depth) { if (depth == 0) { thrower_p(e); return 0; } return through_c(e, depth - 1) + 1; }
int test_main(void) {
    Ex *e = (Ex *)objc_alloc_init(objc_getClass("Ex")); e->code = 5;
    int reached = 0, caught = 0;
    @try { thrower((id)e); reached = 1; } @catch (id x) { caught = 1; check("the caught object is the thrown one", x == (id)e, 0); }
    check("code after the throw is not run", reached == 0, 0);
    check("the handler ran", caught == 1, 0);
    @try { thrower((id)e); } @catch (Ex *x) { value("caught as Ex: code", x->code); }
    @try { thrower((id)e); } @catch (Other *x) { check("an unrelated class does not match", 0, 0); } @catch (id x) { check("falls to the id clause", x == (id)e, 0); }
    @try { thrower((id)e); } @catch (...) { check("catch (...) catches it", 1, 0); }
    ExSub *s = (ExSub *)objc_alloc_init(objc_getClass("ExSub"));
    @try { thrower((id)s); } @catch (Ex *x) { check("a subclass instance matches its superclass clause", x == (Ex *)s, 0); }
    @try { thrower((id)e); } @catch (ExSub *x) { check("a superclass instance does not match a subclass clause", 0, 0); } @catch (id x) { check("so it falls through", 1, 0); }
    @try { @try { thrower((id)e); } @catch (Other *x) { check("inner does not match", 0, 0); } }
    @catch (Ex *x) { check("outer clause catches what the inner did not", x == (Ex *)e, 0); }
    int order = 0, f1 = 0, f2 = 0;
    @try { @try { thrower((id)e); } @finally { f1 = ++order; } } @catch (id x) { f2 = ++order; }
    value("inner @finally runs before the outer handler", f1 == 1 && f2 == 2);
    int fin = 0;
    @try { fin = 0; } @catch (id x) { check("no throw: no handler", 0, 0); } @finally { fin = 1; }
    check("@finally without a throw", fin == 1, 0);
    @try { (void)through_c((id)e, 5); } @catch (id x) { check("through six C frames", x == (id)e, 0); }
    // a class object can be thrown
    @try { thrower((id)objc_getClass("Ex")); } @catch (id x) { check("a class object is thrown and caught", x == (id)objc_getClass("Ex"), 0); }
    // nil can be thrown, and caught by id
    @try { thrower(nil); } @catch (id x) { check("nil thrown, nil caught", x == nil, 0); }
    // throws in a loop do not accumulate state
    int n = 0;
    for (int i = 0; i < 1000; i++) { @try { thrower((id)e); } @catch (id x) { n++; } }
    value("1000 throws caught", n);
    value("retainCount after all of them", (i64)retain_count((id)e));
    rel((id)e); rel((id)s);
    return finish();
}
