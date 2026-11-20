// poly-mac: libobjc
// objc_release: an object released in dealloc of its owner; dealloc order parent first, then children.
#include "poly_mobjc.h"
static int log_[16], n;
@interface Child : NSObject { @public int tag; } @end
@implementation Child
- (void)dealloc { log_[n++] = tag; [super dealloc]; }
@end
@interface Parent : NSObject { @public Child *a, *b; } @end
@implementation Parent
- (void)dealloc {
    log_[n++] = 100;
    objc_release((id)a);
    log_[n++] = 101;
    objc_release((id)b);
    log_[n++] = 102;
    [super dealloc];
    log_[n++] = 103;
}
@end
int test_main(void) {
    Parent *p = [Parent new];
    p->a = [Child new]; p->a->tag = 1;
    p->b = [Child new]; p->b->tag = 2;
    objc_retain((id)p->b);
    objc_release((id)p);
    value("events", n);
    for (int i = 0; i < n; i++) value("event", log_[i]);
    check("the extra reference kept the second child alive", n == 5, n);
    return finish();
}
