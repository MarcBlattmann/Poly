// poly-mac: libobjc
// objc_loadWeakRetained sends -retainWeakReference to the object: a NO answer makes the load
// return nil without changing the location; inside -dealloc the weak reference already reads nil.
#include "poly_mobjc.h"
static int c_rw, deallocs;
static id *wloc; static int nil_in_dealloc = -1;
@interface Cust : NSObject @end
@implementation Cust
- (BOOL)retainWeakReference { c_rw++; return [super retainWeakReference]; }
- (void)dealloc {
    if (wloc) nil_in_dealloc = objc_loadWeakRetained(wloc) == nil;
    deallocs++;
    [super dealloc];
}
@end
@interface Refuse : NSObject @end
@implementation Refuse
- (BOOL)retainWeakReference { return NO; }
@end
int test_main(void) {
    id c = [Cust new];
    id loc = 0;
    objc_storeWeak(&loc, c);
    value("-retainWeakReference calls before any load", c_rw);
    id l = objc_loadWeakRetained(&loc);
    value("-retainWeakReference calls after a load", c_rw);
    check("loaded", l == c, 0);
    objc_release(l);
    wloc = &loc;
    objc_release(c);
    value("deallocs", deallocs);
    value("the weak reference read nil inside -dealloc", nil_in_dealloc);
    wloc = 0;
    id r = [Refuse new];
    id lr = 0;
    objc_storeWeak(&lr, r);
    check("a NO answer: nil returned", objc_loadWeakRetained(&lr) == nil, 0);
    check("a NO answer: the location is unchanged", lr == r, 0);
    value("a NO answer: retainCount unchanged", (i64)retain_count(r));
    objc_release(r);
    check("the location is zeroed when it dies", lr == nil, 0);
    return finish();
}
