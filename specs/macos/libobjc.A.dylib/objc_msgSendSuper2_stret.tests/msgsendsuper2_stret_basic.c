// poly-mac: libobjc
// objc_msgSendSuper2_stret (x86_64 only): objc_msgSendSuper2 for a method
// that returns a struct in memory. Arguments: result buffer, the objc_super
// pair, _cmd, then the method's. The search starts in the superclass of
// super_class; self stays the receiver. Not present on arm64.
#include "poly_mobjc.h"
#include <dlfcn.h>

typedef struct { long a, b, c; } Three;

static id seen_self;

@interface Base : NSObject
- (Three)make:(long)x;
@end
@interface Mid : Base
- (Three)make:(long)x;
@end
@interface Leaf : Mid
@end
@implementation Base
- (Three)make:(long)x { seen_self = self; Three t = {x, 1, 1}; return t; }
@end
@implementation Mid
- (Three)make:(long)x { Three t = [super make:x + 1]; t.b += 10; return t; }   /* compiler-emitted stret super send */
@end
@implementation Leaf
@end

int test_main(void) {
#if defined(__x86_64__)
    extern void objc_msgSendSuper2_stret(void);
    id l = [Leaf new];
    Three t = {0, 0, 0};
    struct objc_super s = {l, objc_getClass("Mid")};
    ((void (*)(Three *, struct objc_super *, SEL, long))objc_msgSendSuper2_stret)(&t, &s, sel_registerName("make:"), 5);
    check("super_class = Mid starts at Base", t.a == 5 && t.b == 1 && t.c == 1, t.a);
    check("self is the receiver", seen_self == l, 0);
    s.super_class = objc_getClass("Leaf");
    ((void (*)(Three *, struct objc_super *, SEL, long))objc_msgSendSuper2_stret)(&t, &s, sel_registerName("make:"), 5);
    check("super_class = Leaf starts at Mid, which calls Base", t.a == 6 && t.b == 11, t.a);
    ((void (*)(Three *, id, SEL, long))objc_msgSend_stret)(&t, l, sel_registerName("make:"), 5);
    check("a plain stret send runs Mid's override", t.a == 6 && t.b == 11, t.a);
    /* A nil receiver is not short-circuited. */
    seen_self = l;
    struct objc_super ns = {(id)0, objc_getClass("Mid")};
    t.a = 0;
    ((void (*)(Three *, struct objc_super *, SEL, long))objc_msgSendSuper2_stret)(&t, &ns, sel_registerName("make:"), 5);
    check("nil receiver: the method ran", t.a == 5, t.a);
    check("with self == nil", seen_self == (id)0, 0);
#else
    check("objc_msgSendSuper2_stret does not exist on arm64", dlsym(RTLD_DEFAULT, "objc_msgSendSuper2_stret") == 0, 0);
#endif
    return finish();
}
