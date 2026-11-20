// poly-mac: libobjc
#include "poly_mobjc.h"
static int dealloc_n;
@interface Cnt : NSObject
@end
@implementation Cnt
- (void)dealloc { dealloc_n++; [super dealloc]; }
@end
int test_main(void) {
    Class c = objc_getClass("Cnt");
    id o = objc_alloc(c);
    check("alloc returns object", o != 0, 0);
    value_str("class", cname(o));
    value("retainCount", (i64)retain_count(o));
    value("instance size", (i64)class_getInstanceSize(c));
    check("distinct objects", objc_alloc(c) != o, 0);
    id p = objc_alloc((Class)objc_getClass("NSObject"));
    value_str("NSObject class", cname(p));
    value("NSObject size", (i64)class_getInstanceSize(objc_getClass("NSObject")));
    check("nil class gives nil", objc_alloc(Nil) == 0, 0);
    id init = send0(o, "init");
    check("init returns same", init == o, 0);
    ((void(*)(id,SEL))objc_msgSend)(o, SEL_("release"));
    value("dealloc ran once", dealloc_n);
    return finish();
}
