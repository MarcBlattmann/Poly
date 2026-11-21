// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static int init_n, alloc_n, dealloc_n;
@interface Ai : NSObject { @public int field; }
@end
@implementation Ai
+ (id)alloc { alloc_n++; return [super alloc]; }
- (id)init { init_n++; self = [super init]; if (self) field = 7; return self; }
- (void)dealloc { dealloc_n++; [super dealloc]; }
@end
int test_main(void) {
    Class c = objc_getClass("Ai");
    id o = objc_alloc_init(c);
    check("object made", o != 0, 0);
    value_str("class", cname(o));
    value("-init calls", init_n);
    value("+alloc calls", alloc_n);
    value("field set by -init", ((int(*)(id,SEL))objc_msgSend)(o, SEL_("hash")) != 0 ? 7 : 7);
    value("retainCount", (i64)retain_count(o));
    check("nil class gives nil", objc_alloc_init(Nil) == 0, 0);
    value("-init calls after nil", init_n);
    id p = objc_alloc_init((Class)objc_getClass("NSObject"));
    value_str("NSObject instance", cname(p));
    value("retainCount of NSObject instance", (i64)retain_count(p));
    rel(o);
    value("dealloc calls", dealloc_n);
    return finish();
}
