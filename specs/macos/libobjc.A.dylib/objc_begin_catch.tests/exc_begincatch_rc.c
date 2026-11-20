// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
@interface Ex : NSObject
@end
@implementation Ex
@end
static id g;
static void thrower(id e) { objc_exception_throw(e); }
static void with_finally(id e) {
    @try { thrower(e); }
    @finally { value("rc in an inner @finally while unwinding", (i64)retain_count(g)); }
}
static void catch_and_rethrow(id e) {
    @try { thrower(e); }
    @catch (id x) { value("rc in the first handler", (i64)retain_count(x)); @throw; }
}
int test_main(void) {
    id e = objc_alloc_init(objc_getClass("Ex")); g = e;
    value("rc before any throw", (i64)retain_count(e));
    @try { thrower(e); } @catch (id x) { value("rc inside @catch (id)", (i64)retain_count(x)); }
    value("rc after @catch", (i64)retain_count(e));
    @try { thrower(e); } @catch (...) { value("rc inside @catch (...)", (i64)retain_count(e)); }
    value("rc after @catch (...)", (i64)retain_count(e));
    @try { with_finally(e); } @catch (id x) { value("rc in the handler after the @finally", (i64)retain_count(x)); }
    value("rc after that", (i64)retain_count(e));
    @try { catch_and_rethrow(e); } @catch (id x) { value("rc in the handler of a rethrown exception", (i64)retain_count(x)); check("rethrown is the same object", x == e, 0); }
    value("rc after the rethrow was caught", (i64)retain_count(e));
    @try {
        @try { thrower(e); }
        @catch (id a) {
            value("rc in the outer handler", (i64)retain_count(a));
            @try { thrower(e); }
            @catch (id b) { value("rc in the nested handler (same object thrown again)", (i64)retain_count(b)); }
            value("rc back in the outer handler", (i64)retain_count(a));
        }
    } @catch (id z) { check("nothing escapes", 0, 0); }
    value("rc after the nested handlers", (i64)retain_count(e));
    rel(e);
    return finish();
}
