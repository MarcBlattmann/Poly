// poly-mac: libobjc
// objc_msgSendSuper2 passes the method's arguments through unchanged:
// ten integer arguments (the last ones on the stack), mixed floating
// point arguments, small structs returned in registers, and (arm64) a large
// struct returned through the hidden result pointer.
#include "poly_mobjc.h"

extern void objc_msgSendSuper2(void);

typedef struct { long a, b; } Two;
typedef struct { double x, y; } DD;
typedef struct { long a, b, c, d; } Big;

@interface Base : NSObject
- (long)ten:(long)a :(long)b :(long)c :(long)d :(long)e :(long)f :(long)g :(long)h :(long)i :(long)j;
- (double)fl:(double)a :(float)b :(int)c :(double)d :(double)e :(double)f :(double)g :(double)h :(double)i :(double)j :(double)k;
- (Two)two:(long)a;
- (DD)dd:(double)a;
- (Big)big:(long)a;
@end
@interface Leaf : Base
@end
@implementation Base
- (long)ten:(long)a :(long)b :(long)c :(long)d :(long)e :(long)f :(long)g :(long)h :(long)i :(long)j {
    return a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f + 7 * g + 8 * h + 9 * i + 10 * j;
}
- (double)fl:(double)a :(float)b :(int)c :(double)d :(double)e :(double)f :(double)g :(double)h :(double)i :(double)j :(double)k {
    return a + b + c + 2 * d + 3 * e + 4 * f + 5 * g + 6 * h + 7 * i + 8 * j + 9 * k;
}
- (Two)two:(long)a { Two t = {a, a + 1}; return t; }
- (DD)dd:(double)a { DD t = {a, a * 2}; return t; }
- (Big)big:(long)a { Big b = {a, a + 1, a + 2, a + 3}; return b; }
@end
@implementation Leaf
@end

int test_main(void) {
    id l = [Leaf new];
    struct objc_super s = {l, objc_getClass("Leaf")};
    long v = ((long (*)(struct objc_super *, SEL, long, long, long, long, long, long, long, long, long, long))objc_msgSendSuper2)(
        &s, sel_registerName("ten::::::::::"), 1, 2, 3, 4, 5, 6, 7, 8, 9, 10);
    check("ten integer arguments", v == 385, v);
    double d = ((double (*)(struct objc_super *, SEL, double, float, int, double, double, double, double, double, double, double, double))objc_msgSendSuper2)(
        &s, sel_registerName("fl:::::::::::"), 1.0, 2.0f, 3, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0);
    check("eleven mixed floating arguments", d == 1 + 2 + 3 + 8 + 15 + 24 + 35 + 48 + 63 + 80 + 99, (i64)d);
    Two t = ((Two (*)(struct objc_super *, SEL, long))objc_msgSendSuper2)(&s, sel_registerName("two:"), 40);
    check("two-word struct in registers", t.a == 40 && t.b == 41, t.a);
    DD q = ((DD (*)(struct objc_super *, SEL, double))objc_msgSendSuper2)(&s, sel_registerName("dd:"), 1.5);
    check("two-double struct", q.x == 1.5 && q.y == 3.0, 0);
#if defined(__arm64__)
    Big b = ((Big (*)(struct objc_super *, SEL, long))objc_msgSendSuper2)(&s, sel_registerName("big:"), 10);
    check("arm64: large struct through the result pointer", b.a == 10 && b.d == 13, b.a);
#else
    check("x86_64: large structs use objc_msgSendSuper2_stret", 1, 0);
#endif
    return finish();
}
