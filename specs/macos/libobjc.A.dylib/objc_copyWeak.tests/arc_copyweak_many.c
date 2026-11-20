// poly-mac: libobjc
// objc_copyWeak: copies of copies; every copy is zeroed when the object dies.
#include "poly_mobjc.h"
@interface Obj : NSObject @end
@implementation Obj
@end
int test_main(void) {
    id o = [Obj new];
    id w[20];
    w[0] = 0;
    objc_initWeak(&w[0], o);
    for (int i = 1; i < 20; i++) { w[i] = (id)0x1; objc_copyWeak(&w[i], &w[i - 1]); }
    int same = 0; for (int i = 0; i < 20; i++) same += w[i] == o;
    value("copies holding the object", same);
    objc_release(o);
    int zero = 0; for (int i = 0; i < 20; i++) zero += w[i] == nil;
    value("copies zeroed", zero);
    return finish();
}
