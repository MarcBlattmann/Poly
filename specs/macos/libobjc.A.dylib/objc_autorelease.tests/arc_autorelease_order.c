// poly-mac: libobjc
// objc_autorelease: a pop releases in reverse order of autoreleasing; inner pools first; more than a page of entries.
#include "poly_mobjc.h"
static int deallocs, order[8], n;
@interface Obj : NSObject { @public int tag; } @end
@implementation Obj
- (void)dealloc { deallocs++; if (n < 8) order[n++] = tag; [super dealloc]; }
@end
static Obj *mk(int t) { Obj *o = [Obj new]; o->tag = t; return o; }
@interface Bulk : NSObject @end
static int bulk;
@implementation Bulk
- (void)dealloc { bulk++; [super dealloc]; }
@end
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    objc_autorelease((id)mk(10)); objc_autorelease((id)mk(11)); objc_autorelease((id)mk(12));
    objc_autoreleasePoolPop(p);
    value("events", n);
    for (int i = 0; i < n; i++) value("order", order[i]);
    // the same object autoreleased twice, retained twice
    n = 0; deallocs = 0;
    Obj *b = mk(30);
    objc_retain((id)b);
    p = objc_autoreleasePoolPush();
    objc_autorelease((id)b); objc_autorelease((id)b);
    objc_autoreleasePoolPop(p);
    value("double autorelease: deallocs", deallocs);
    // many
    p = objc_autoreleasePoolPush();
    for (int i = 0; i < 5000; i++) objc_autorelease([Bulk new]);
    value("bulk before pop", bulk);
    objc_autoreleasePoolPop(p);
    value("bulk after pop", bulk);
    return finish();
}
