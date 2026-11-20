// poly-mac: libobjc
// objc_retain: many retains; the object lives until the matching releases; retain during dealloc.
#include "poly_mobjc.h"
static int deallocs, rc_in_dealloc, rc_after_retain_in_dealloc;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
@interface Late : NSObject @end
@implementation Late
- (void)dealloc {
    rc_in_dealloc = (int)retain_count(self);
    id r = objc_retain(self);
    rc_after_retain_in_dealloc = (int)retain_count(self);
    value("retain in dealloc returns self", r == self);
    [super dealloc];
}
@end
int test_main(void) {
    id o = [Obj new];
    for (int i = 0; i < 1000; i++) objc_retain(o);
    value("retainCount after 1000 retains", (i64)retain_count(o));
    for (int i = 0; i < 1000; i++) objc_release(o);
    value("retainCount back", (i64)retain_count(o));
    value("deallocs while 1 reference", deallocs);
    objc_release(o);
    value("deallocs at the end", deallocs);
    // retain and release in different order
    id a = [Obj new], b = [Obj new];
    objc_retain(a); objc_retain(b);
    objc_release(a); objc_release(b);
    value("both still alive", deallocs);
    objc_release(a); objc_release(b);
    value("both gone", deallocs);
    id l = [Late new];
    objc_release(l);
    value("retainCount seen inside dealloc", rc_in_dealloc);
    value("retainCount after a retain inside dealloc", rc_after_retain_in_dealloc);
    return finish();
}
