// poly-mac: libobjc
// poly-mac: arc
// objc_storeWeak, objc_initWeak, objc_loadWeakRetained and objc_destroyWeak as ARC-compiled code uses
// them: __weak locals, globals and instance variables read nil once the object is gone; a weak read
// gives a strong reference that keeps the object alive while held; a __weak local goes out of scope
// without disturbing the object.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; }
@end
@interface Holder : NSObject { @public __weak Obj *w; } @end
@implementation Holder @end
static __weak Obj *gw;
__attribute__((noinline)) static Obj *peek_global(void) { return gw; }
__attribute__((noinline)) static void make_weak_in_scope(void) {
    Obj *o = [Obj new];
    __weak Obj *w = o;
    value("a weak local reads the object", w != nil);
}
int test_main(void) {
    Obj *o = [Obj new];
    gw = o;
    Holder *h = [Holder new];
    h->w = o;
    __weak Obj *local = o;
    value("the global reads the object", gw != nil);
    value("the ivar reads the object", h->w != nil);
    value("the local reads the object", local != nil);
    {
        Obj *strong = peek_global();
        value("a weak read gives a strong reference", strong == o);
        o = nil;
        value("deallocs while the weak read result is held", deallocs);
    }
    value("deallocs once it is dropped", deallocs);
    value("the global is nil", gw == nil);
    value("the ivar is nil", h->w == nil);
    value("the local is nil", local == nil);
    make_weak_in_scope();
    value("deallocs after a function with a weak local", deallocs);
    return finish();
}
