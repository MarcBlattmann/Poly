// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static int dealloc_n;
@interface V : NSObject
@end
@implementation V
- (void)dealloc { dealloc_n++; [super dealloc]; }
@end
static char k1, k2, k3, k4, k5;
int test_main(void) {
    void *outer = objc_autoreleasePoolPush();
    id owner = new_object();
    id v = objc_alloc_init(objc_getClass("V"));
    check("none yet", objc_getAssociatedObject(owner, &k1) == 0, 0);
    // ASSIGN
    objc_setAssociatedObject(owner, &k1, v, OBJC_ASSOCIATION_ASSIGN);
    check("assign: get returns it", objc_getAssociatedObject(owner, &k1) == v, 0);
    value("assign: retainCount", (i64)retain_count(v));
    // RETAIN_NONATOMIC
    objc_setAssociatedObject(owner, &k2, v, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    value("retain_nonatomic: retainCount", (i64)retain_count(v));
    check("retain_nonatomic: get returns it", objc_getAssociatedObject(owner, &k2) == v, 0);
    // RETAIN (atomic): get autoreleases
    objc_setAssociatedObject(owner, &k3, v, OBJC_ASSOCIATION_RETAIN);
    value("retain: retainCount after set", (i64)retain_count(v));
    check("retain: get returns it", objc_getAssociatedObject(owner, &k3) == v, 0);
    void *pool = objc_autoreleasePoolPush();
    objc_getAssociatedObject(owner, &k3);
    value("retain: retainCount after get inside pool", (i64)retain_count(v));
    objc_autoreleasePoolPop(pool);
    value("retain: retainCount after pool popped", (i64)retain_count(v));
    // nonatomic get does not autorelease
    pool = objc_autoreleasePoolPush();
    objc_getAssociatedObject(owner, &k2);
    value("nonatomic: retainCount after get inside pool", (i64)retain_count(v));
    objc_autoreleasePoolPop(pool);
    value("nonatomic: retainCount after pool popped", (i64)retain_count(v));
    // remove by setting nil
    objc_setAssociatedObject(owner, &k3, nil, OBJC_ASSOCIATION_RETAIN);
    value("retainCount after nil on k3", (i64)retain_count(v));
    check("k3 gone", objc_getAssociatedObject(owner, &k3) == 0, 0);
    objc_setAssociatedObject(owner, &k2, nil, OBJC_ASSOCIATION_ASSIGN);
    value("retainCount after nil on k2 (policy of the nil call differs)", (i64)retain_count(v));
    check("k2 gone", objc_getAssociatedObject(owner, &k2) == 0, 0);
    check("k1 still there", objc_getAssociatedObject(owner, &k1) == v, 0);
    (void)k4; (void)k5;
    value("dealloc so far", dealloc_n);
    rel(v);
    value("dealloc after our release (the leaked autorelease of the first get is still pending)", dealloc_n);
    objc_autoreleasePoolPop(outer);
    value("dealloc after the outer pool is popped", dealloc_n);
    rel(owner);
    return finish();
}
