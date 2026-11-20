// poly-mac: libobjc
// objc_storeStrong sends -retain to the new object and -release to the old one when their class overrides them;
// the old value is released after the new one was stored (a dealloc that reads the slot sees the new value).
#include "poly_mobjc.h"
static int c_retain, c_release;
static id *watch; static id seen;
@interface Cust : NSObject @end
@implementation Cust
- (id)retain { c_retain++; return [super retain]; }
- (oneway void)release { c_release++; [super release]; }
- (void)dealloc { if (watch) seen = *watch; [super dealloc]; }
@end
int test_main(void) {
    id a = [Cust new], b = [Cust new];
    id slot = 0;
    objc_storeStrong(&slot, a);
    value("-retain calls", c_retain); value("-release calls", c_release);
    objc_storeStrong(&slot, b);
    value("-retain calls after replacing", c_retain); value("-release calls after replacing", c_release);
    objc_release(a);
    watch = &slot;
    objc_storeStrong(&slot, nil);   // b still owned by the test
    watch = 0;
    objc_release(b);
    value("-release calls at the end", c_release);
    // dealloc of the old value sees the new value in the slot
    id x = [Cust new], y = [Cust new];
    id slot2 = 0;
    objc_storeStrong(&slot2, x);
    objc_release(x);               // slot2 sole owner
    watch = &slot2; seen = 0;
    objc_storeStrong(&slot2, y);
    watch = 0;
    check("the old object's dealloc saw the new value already stored", seen == y, 0);
    objc_release(y);
    objc_storeStrong(&slot2, nil);
    return finish();
}
