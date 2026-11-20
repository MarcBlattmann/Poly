// poly-mac: libobjc
// objc_storeWeak: a location that held another object is unregistered from it; the old object's
// deallocation does not touch the location any more.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
int test_main(void) {
    id a = [Obj new], b = [Obj new];
    id s = 0, s2 = 0;
    objc_storeWeak(&s, a);
    objc_storeWeak(&s2, a);
    id r = objc_storeWeak(&s, b);
    check("returns the new object", r == b, 0);
    check("location holds the new object", s == b, 0);
    objc_release(a);
    value("deallocs after a is gone", deallocs);
    check("s still refers to b", s == b, 0);
    check("s2 was zeroed", s2 == nil, 0);
    // same object again
    id r2 = objc_storeWeak(&s, b);
    check("storing the same object again", r2 == b && s == b, 0);
    objc_release(b);
    check("s zeroed once b is gone", s == nil, 0);
    value("deallocs", deallocs);
    // many locations on one object
    id c = [Obj new];
    id locs[50];
    for (int i = 0; i < 50; i++) { locs[i] = 0; objc_storeWeak(&locs[i], c); }
    int set = 0; for (int i = 0; i < 50; i++) set += locs[i] == c;
    value("locations set", set);
    objc_release(c);
    int zero = 0; for (int i = 0; i < 50; i++) zero += locs[i] == nil;
    value("locations zeroed", zero);
    return finish();
}
