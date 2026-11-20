// poly-mac: libobjc
// objc_msgSendSuper2: sends a message to `receiver` but starts the method
// search in the superclass of `super_class` (the class the calling code
// belongs to), whatever the receiver's own class is. self stays the
// receiver. The compiler emits this call for [super ...].
#include "poly_mobjc.h"

extern void objc_msgSendSuper2(void);

static id seen_self;
static SEL seen_cmd;

@interface Base : NSObject
- (int)who;
- (long)add:(long)a to:(long)b;
- (double)scale:(double)a by:(float)f;
+ (int)cwho;
@end
@interface Mid : Base
- (int)who;
- (long)add:(long)a to:(long)b;
- (double)scale:(double)a by:(float)f;
+ (int)cwho;
@end
@interface Leaf : Mid
- (int)who;
@end
@implementation Base
- (int)who { seen_self = self; seen_cmd = _cmd; return 1; }
- (long)add:(long)a to:(long)b { return a + b; }
- (double)scale:(double)a by:(float)f { return a * f; }
+ (int)cwho { seen_self = (id)self; return 10; }
@end
@implementation Mid
- (int)who { return 20 + [super who]; }                       /* compiler-emitted super send */
- (long)add:(long)a to:(long)b { return 1000 + [super add:a to:b]; }
- (double)scale:(double)a by:(float)f { return 1.0 + [super scale:a by:f]; }
+ (int)cwho { return 200 + [super cwho]; }                    /* in a class method */
@end
@implementation Leaf
- (int)who { return 300 + [super who]; }
@end

int test_main(void) {
    Class base = objc_getClass("Base"), mid = objc_getClass("Mid"), leaf = objc_getClass("Leaf");
    id l = [Leaf new];
    SEL who = sel_registerName("who");

    /* Hand-built super sends. The receiver is a Leaf in every case. */
    struct objc_super s = {l, mid};
    check("super_class = Mid starts at Base", ((int (*)(struct objc_super *, SEL))objc_msgSendSuper2)(&s, who) == 1, 0);
    check("self is the receiver", seen_self == l, 0);
    check("_cmd is the selector", seen_cmd == who, 0);
    s.super_class = leaf;
    check("super_class = Leaf starts at Mid (which calls Base)", ((int (*)(struct objc_super *, SEL))objc_msgSendSuper2)(&s, who) == 21, 0);
    s.super_class = base;
    /* Base's superclass is NSObject, which has no -who: not sent (it would
     * be an unrecognized selector). Use a selector NSObject has. */
    check("super_class = Base reaches NSObject's -self", ((id (*)(struct objc_super *, SEL))objc_msgSendSuper2)(&s, sel_registerName("self")) == l, 0);

    /* Arguments and other return types pass through. */
    s.super_class = leaf;
    long a = ((long (*)(struct objc_super *, SEL, long, long))objc_msgSendSuper2)(&s, sel_registerName("add:to:"), 3, 4);
    check("integer arguments (Mid's override adds 1000 to Base's)", a == 1007, a);
    s.super_class = mid;
    a = ((long (*)(struct objc_super *, SEL, long, long))objc_msgSendSuper2)(&s, sel_registerName("add:to:"), 3, 4);
    check("integer arguments, starting at Base", a == 7, a);
    double d = ((double (*)(struct objc_super *, SEL, double, float))objc_msgSendSuper2)(&s, sel_registerName("scale:by:"), 3.0, 2.0f);
    check("floating arguments and return", d == 6.0, 0);

    /* Calls the compiler wrote: the same chain from the receiver's class. */
    check("[l who] runs Leaf, Mid, Base", ((int (*)(id, SEL))objc_msgSend)(l, who) == 321, 0);
    check("Mid's [super add:to:]", ((long (*)(id, SEL, long, long))objc_msgSend)(l, sel_registerName("add:to:"), 1, 2) == 1003, 0);
    check("Mid's [super scale:by:]", ((double (*)(id, SEL, double, float))objc_msgSend)(l, sel_registerName("scale:by:"), 2.0, 4.0f) == 9.0, 0);

    /* A class method: the receiver is a class, super_class is the metaclass
     * of the calling class. */
    struct objc_super cs = {(id)leaf, object_getClass((id)mid)};
    check("class method super send starts at Base's metaclass", ((int (*)(struct objc_super *, SEL))objc_msgSendSuper2)(&cs, sel_registerName("cwho")) == 10, 0);
    check("self is the class", seen_self == (id)leaf, 0);
    check("+cwho through Mid's [super cwho]", ((int (*)(id, SEL))objc_msgSend)((id)leaf, sel_registerName("cwho")) == 210, 0);

    /* A nil receiver is NOT short-circuited as objc_msgSend does: the
     * method is found and called with self == nil. */
    seen_self = l;
    struct objc_super ns = {(id)0, mid};
    int r = ((int (*)(struct objc_super *, SEL))objc_msgSendSuper2)(&ns, who);
    value("result with a nil receiver", r);
    check("the method ran (it returned 1)", r == 1, r);
    check("with self == nil", seen_self == (id)0, 0);
    return finish();
}
