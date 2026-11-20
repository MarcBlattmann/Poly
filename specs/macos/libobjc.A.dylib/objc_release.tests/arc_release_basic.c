// poly-mac: libobjc
// objc_release: the last release sends -dealloc once; nil is accepted; no return value.
#include "poly_mobjc.h"
static int deallocs, order[8], n;
@interface Obj : NSObject { @public int tag; } @end
@implementation Obj
- (void)dealloc { deallocs++; order[n++] = tag; [super dealloc]; }
@end
int test_main(void) {
    Obj *a = [Obj new]; a->tag = 1;
    Obj *b = [Obj new]; b->tag = 2;
    objc_retain((id)a);
    objc_release((id)a);
    value("deallocs after a non-final release", deallocs);
    objc_release((id)a);
    value("deallocs after the final release", deallocs);
    objc_release((id)b);
    value("deallocs after the second object", deallocs);
    value("order[0]", order[0]); value("order[1]", order[1]);
    objc_release(nil);
    objc_release(0);
    value("nil release survived", 1);
    value("deallocs unchanged by nil", deallocs);
    return finish();
}
