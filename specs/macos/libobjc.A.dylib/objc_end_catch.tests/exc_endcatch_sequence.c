// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static int live, made, gone;
@interface Ex : NSObject
@end
@implementation Ex
- (id)init { self = [super init]; if (self) { live++; made++; } return self; }
- (void)dealloc { live--; gone++; [super dealloc]; }
@end
static void thrower(id e) { objc_exception_throw(e); }
static int maxlive;
int test_main(void) {
    for (int i = 0; i < 1000; i++) {
        id e = objc_alloc_init(objc_getClass("Ex"));
        @try { thrower(e); }
        @catch (id x) { rel(x); if (live > maxlive) maxlive = live; }
    }
    value("objects made", made);
    value("objects deallocated by the end of the loop", gone);
    value("live at the end", live);
    value("most live at once", maxlive);
    // many nested handlers, 50 deep, each holding its own exception
    int depth = 0;
    id es[50];
    for (int i = 0; i < 50; i++) es[i] = objc_alloc_init(objc_getClass("Ex"));
    // build the nesting by hand with a loop of recursion
    extern void nest(id *es, int i, int *depth);
    int before = live;
    nest(es, 0, &depth);
    value("deepest nesting reached", depth);
    value("live objects after the nested handlers ended (all released inside)", live - before + 50 - 50);
    value("gone", gone);
    return finish();
}
void nest(id *es, int i, int *depth) {
    if (i == 50) return;
    @try { thrower(es[i]); }
    @catch (id x) {
        *depth = i + 1;
        nest(es, i + 1, depth);
        rel(x);
    }
}
