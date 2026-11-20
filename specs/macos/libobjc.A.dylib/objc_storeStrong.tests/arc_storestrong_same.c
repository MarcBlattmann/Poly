// poly-mac: libobjc
// objc_storeStrong: storing the object the slot already holds changes no count and never deallocates;
// the last reference being the slot's, storing nil deallocates.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id slot = 0;
    objc_storeStrong(&slot, o);
    objc_release(o);               // the slot is the only owner now
    value("retainCount, the slot the only owner", (i64)retain_count(slot));
    objc_storeStrong(&slot, slot);
    value("retainCount after storing the same object", (i64)retain_count(slot));
    value("deallocs after storing the same object", deallocs);
    check("slot unchanged", slot == o, 0);
    objc_storeStrong(&slot, nil);
    value("deallocs after storing nil", deallocs);
    check("slot nil", slot == nil, 0);
    objc_storeStrong(&slot, nil);
    value("nil over nil survived", 1);
    return finish();
}
