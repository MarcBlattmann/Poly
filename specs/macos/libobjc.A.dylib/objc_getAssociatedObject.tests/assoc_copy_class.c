// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static int copies, deallocs, seen_in_dealloc;
static char key, ckey, klass;
@interface Cp : NSObject { @public int gen; }
@end
@implementation Cp
- (id)copyWithZone:(struct _NSZone *)z { copies++; Cp *c = (Cp *)[[self class] allocWithZone:z]; c->gen = gen + 1; return (id)c; }
- (void)dealloc { deallocs++; [super dealloc]; }
@end
@interface Ow : NSObject
@end
@implementation Ow
- (void)dealloc { seen_in_dealloc = objc_getAssociatedObject(self, &ckey) != 0; [super dealloc]; }
@end
int test_main(void) {
    void *pool = objc_autoreleasePoolPush();
    Cp *v = (Cp *)objc_alloc_init(objc_getClass("Cp"));
    id owner = new_object();
    objc_setAssociatedObject(owner, &key, (id)v, OBJC_ASSOCIATION_COPY_NONATOMIC);
    value("copyWithZone: calls after COPY_NONATOMIC set", copies);
    id got = objc_getAssociatedObject(owner, &key);
    check("the stored value is a copy, not the original", got != (id)v, 0);
    value("the copy's generation", ((Cp *)got)->gen);
    value("original retainCount unchanged", (i64)retain_count((id)v));
    value("copy retainCount", (i64)retain_count(got));
    value("copyWithZone: calls after get", copies);
    objc_setAssociatedObject(owner, &ckey, (id)v, OBJC_ASSOCIATION_COPY);
    value("copyWithZone: calls after COPY set", copies);
    check("COPY get returns the stored copy", objc_getAssociatedObject(owner, &ckey) != (id)v, 0);
    objc_setAssociatedObject(owner, &ckey, nil, OBJC_ASSOCIATION_COPY);
    value("copyWithZone: calls after setting nil with COPY", copies);
    // associations on a class object
    Class c = objc_getClass("Ow");
    objc_setAssociatedObject((id)c, &klass, (id)v, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    check("class object holds one", objc_getAssociatedObject((id)c, &klass) == (id)v, 0);
    check("its metaclass does not", objc_getAssociatedObject(object_getClass((id)c), &klass) == 0, 0);
    check("an instance does not", objc_getAssociatedObject(owner, &klass) == 0, 0);
    objc_setAssociatedObject((id)c, &klass, nil, OBJC_ASSOCIATION_ASSIGN);
    // visible from the owner's dealloc
    id ow = objc_alloc_init(c);
    objc_setAssociatedObject(ow, &ckey, (id)v, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    rel(ow);
    value("association still readable inside the owner's -dealloc", seen_in_dealloc);
    rel(owner);
    objc_autoreleasePoolPop(pool);
    value("deallocs of Cp objects (the copy)", deallocs);
    rel((id)v);
    value("deallocs of Cp objects after releasing the original", deallocs);
    return finish();
}
