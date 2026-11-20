// poly-mac: libobjc
// objc_autorelease: returns its argument, defers the release to the pool's pop; nil is accepted.
#include "poly_mobjc.h"
static int deallocs, order[16], n;
@interface Obj : NSObject { @public int tag; } @end
@implementation Obj
- (void)dealloc { deallocs++; order[n++] = tag; [super dealloc]; }
@end
static Obj *mk(int t) { Obj *o = [Obj new]; o->tag = t; return o; }
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    Obj *a = mk(1);
    id r = objc_autorelease((id)a);
    check("returns its argument", r == (id)a, 0);
    value("retainCount unchanged by autorelease", (i64)retain_count((id)a));
    value("deallocs before pop", deallocs);
    objc_autoreleasePoolPop(p);
    value("deallocs after pop", deallocs);
    check("nil in, nil out", objc_autorelease(nil) == nil, 0);
    // retained object survives
    p = objc_autoreleasePoolPush();
    Obj *b = mk(2);
    objc_retain((id)b);
    objc_autorelease((id)b);
    objc_autoreleasePoolPop(p);
    value("deallocs: retained object survives", deallocs);
    value("retainCount", (i64)retain_count((id)b));
    objc_release((id)b);
    value("deallocs after the explicit release", deallocs);
    return finish();
}
