// poly-mac: libobjc
// objc_storeStrong: retains the new value, releases the old one, stores the new value; no return value.
#include "poly_mobjc.h"
static int deallocs, order[8], n;
@interface Obj : NSObject { @public int tag; } @end
@implementation Obj
- (void)dealloc { deallocs++; order[n++] = tag; [super dealloc]; }
@end
static Obj *mk(int t) { Obj *o = [Obj new]; o->tag = t; return o; }
int test_main(void) {
    Obj *s1 = mk(1), *s2 = mk(2);
    id slot = 0;
    objc_storeStrong(&slot, (id)s1);
    check("slot holds the object", slot == (id)s1, 0);
    value("retainCount of the first after the store", (i64)retain_count((id)s1));
    objc_storeStrong(&slot, (id)s2);
    check("slot holds the second", slot == (id)s2, 0);
    value("retainCount of the first after it was replaced", (i64)retain_count((id)s1));
    value("retainCount of the second", (i64)retain_count((id)s2));
    objc_release((id)s1);
    value("deallocs after the owner released the first", deallocs);
    objc_storeStrong(&slot, nil);
    check("slot is nil", slot == nil, 0);
    value("retainCount of the second after the slot was cleared", (i64)retain_count((id)s2));
    objc_release((id)s2);
    value("deallocs", deallocs);
    value("order[0]", order[0]); value("order[1]", order[1]);
    return finish();
}
