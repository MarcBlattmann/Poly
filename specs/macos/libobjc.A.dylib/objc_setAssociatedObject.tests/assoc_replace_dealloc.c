// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static int order[8], n;
@interface Val : NSObject { @public int tag; }
@end
@implementation Val
- (void)dealloc { if (n < 8) order[n++] = tag; [super dealloc]; }
@end
@interface Own : NSObject { @public int tag; }
@end
@implementation Own
- (void)dealloc { if (n < 8) order[n++] = 100 + tag; [super dealloc]; }
@end
static char ka, kb;
static id mk(int t) { Val *v = (Val *)objc_alloc_init(objc_getClass("Val")); v->tag = t; return (id)v; }
int test_main(void) {
    Own *owner = (Own *)objc_alloc_init(objc_getClass("Own"));
    owner->tag = 1;
    id a = mk(1), b = mk(2), c = mk(3);
    objc_setAssociatedObject((id)owner, &ka, a, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    rel(a);
    value("owns a: retainCount", (i64)retain_count(a));
    objc_setAssociatedObject((id)owner, &ka, b, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    value("a released when replaced: dealloc count", n);
    value("a's tag first in the dealloc log", n ? order[0] : -1);
    value("b retainCount (ours + the association)", (i64)retain_count(b));
    rel(b);
    check("get returns the new value", objc_getAssociatedObject((id)owner, &ka) == b, 0);
    // same value set again with RETAIN: not freed
    objc_setAssociatedObject((id)owner, &ka, b, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    value("same value set again: dealloc count", n);
    value("same value set again: retainCount", (i64)retain_count(b));
    objc_setAssociatedObject((id)owner, &kb, c, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    rel(c);
    n = 0;
    rel((id)owner);
    value("deallocs when the owner goes", n);
    // the log: values are released in some order relative to the owner's dealloc
    int owner_pos = -1, vals = 0;
    for (int i = 0; i < n; i++) { if (order[i] == 101) owner_pos = i; else vals++; }
    value("owner position in the dealloc log", owner_pos);
    value("values released", vals);
    return finish();
}
