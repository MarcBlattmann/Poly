// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static int deallocs;
@interface Ex : NSObject
@end
@implementation Ex
- (void)dealloc { deallocs++; [super dealloc]; }
@end
static void thrower(id e) { objc_exception_throw(e); }
static int via_return(id e) { @try { thrower(e); } @catch (id x) { rel(x); return 11; } return 0; }
static int via_goto(id e) { @try { thrower(e); } @catch (id x) { rel(x); goto out; } return 0; out: return 22; }
static int via_finally_return(id e) { @try { thrower(e); } @catch (id x) { rel(x); } @finally { return 33; } }
int test_main(void) {
    int d = deallocs;
    id e = objc_alloc_init(objc_getClass("Ex"));
    value("return from inside a handler", via_return(e));
    value("deallocs: the runtime's reference went with the return", deallocs - d);
    d = deallocs; e = objc_alloc_init(objc_getClass("Ex"));
    value("goto out of a handler", via_goto(e));
    value("deallocs after goto", deallocs - d);
    d = deallocs; e = objc_alloc_init(objc_getClass("Ex"));
    value("return from @finally after a handler", via_finally_return(e));
    value("deallocs after @finally return", deallocs - d);
    // exception thrown again inside a handler: both are released
    d = deallocs;
    id a = objc_alloc_init(objc_getClass("Ex")), b = objc_alloc_init(objc_getClass("Ex"));
    @try {
        @try { thrower(a); }
        @catch (id x) { rel(x); thrower(b); }
    } @catch (id y) { rel(y); value("deallocs inside the outer handler (a is gone, b is held)", deallocs - d); }
    value("deallocs after both", deallocs - d);
    // an exception thrown from a @finally while another is propagating replaces it
    d = deallocs;
    id c = objc_alloc_init(objc_getClass("Ex")), f = objc_alloc_init(objc_getClass("Ex"));
    @try {
        @try { thrower(c); }
        @finally { @try { thrower(f); } @catch (id q) { rel(q); } }
    } @catch (id z) { check("the original exception arrives", z == c, 0); rel(z); }
    value("deallocs for the replaced pair", deallocs - d);
    return finish();
}
