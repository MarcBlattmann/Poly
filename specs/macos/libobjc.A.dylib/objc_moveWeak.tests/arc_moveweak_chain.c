// poly-mac: libobjc
// objc_moveWeak: moving many references of one object into an array leaves the sources nil and the
// destinations following the object; a moved location may be destroyed.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id o = [Obj new];
    id src[10], dst[10];
    for (int i = 0; i < 10; i++) { src[i] = 0; objc_initWeak(&src[i], o); dst[i] = (id)0x1; }
    for (int i = 0; i < 10; i++) objc_moveWeak(&dst[i], &src[i]);
    int s = 0, d = 0;
    for (int i = 0; i < 10; i++) { s += src[i] == nil; d += dst[i] == o; }
    value("sources nil", s); value("destinations holding the object", d);
    value("retainCount", (i64)retain_count(o));
    objc_destroyWeak(&dst[3]);
    dst[3] = (id)0x99;
    objc_release(o);
    value("deallocs", deallocs);
    int z = 0; for (int i = 0; i < 10; i++) z += dst[i] == nil;
    value("destinations zeroed", z);
    check("the destroyed one was left alone", dst[3] == (id)0x99, 0);
    return finish();
}
