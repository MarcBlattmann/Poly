// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static int allocs, inits, deallocs;
@interface Ar : NSObject
@end
@implementation Ar
+ (id)alloc { allocs++; return [super alloc]; }
- (id)init { inits++; return [super init]; }
- (void)dealloc { deallocs++; [super dealloc]; }
@end
@interface Ar2 : Ar
@end
@implementation Ar2
@end
int test_main(void) {
    Ar *a = [[Ar alloc] init];
    check("[[C alloc] init] makes an instance", a != nil, 0);
    value("+alloc calls", allocs);
    value("-init calls", inits);
    Ar *b = [Ar new];
    value("+alloc calls after +new", allocs);
    value("-init calls after +new", inits);
    id c = objc_alloc((Class)[Ar2 class]);
    id d = objc_alloc_init((Class)[Ar2 class]);
    value("+alloc calls after objc_alloc and objc_alloc_init on a subclass", allocs);
    value("-init calls after those", inits);
    value_str("class of d", cname(d));
    check("c and d are distinct", c != d, 0);
    rel((id)a); rel((id)b); rel(c); rel(d);
    value("deallocs after four releases", deallocs);
    return finish();
}
