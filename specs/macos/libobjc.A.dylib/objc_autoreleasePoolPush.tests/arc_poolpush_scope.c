// poly-mac: libobjc
// objc_autoreleasePoolPush: objects autoreleased after a push belong to that pool, objects
// autoreleased before it to the outer pool; an empty pool pops without effect.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    void *outer = objc_autoreleasePoolPush();
    objc_autorelease([Obj new]);
    void *inner = objc_autoreleasePoolPush();
    objc_autorelease([Obj new]);
    objc_autorelease([Obj new]);
    value("deallocs before any pop", deallocs);
    objc_autoreleasePoolPop(inner);
    value("deallocs after the inner pop", deallocs);
    objc_autoreleasePoolPop(outer);
    value("deallocs after the outer pop", deallocs);
    void *e = objc_autoreleasePoolPush();
    objc_autoreleasePoolPop(e);
    value("an empty pool popped without effect", deallocs);
    // an object autoreleased after the inner pool was popped goes to the outer one again
    deallocs = 0;
    outer = objc_autoreleasePoolPush();
    inner = objc_autoreleasePoolPush();
    objc_autoreleasePoolPop(inner);
    objc_autorelease([Obj new]);
    value("deallocs after popping the inner and autoreleasing", deallocs);
    objc_autoreleasePoolPop(outer);
    value("deallocs after the outer pop (second)", deallocs);
    return finish();
}
