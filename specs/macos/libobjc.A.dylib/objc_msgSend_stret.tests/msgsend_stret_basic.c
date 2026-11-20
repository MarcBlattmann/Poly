// poly-mac: libobjc
// objc_msgSend_stret (x86_64 only): the call for a method that returns a
// struct in memory (more than 16 bytes). The first argument is the address
// of the result buffer, then self, _cmd and the method's arguments. The
// symbol does not exist in the arm64 runtime, where objc_msgSend does it all.
#include "poly_mobjc.h"
#include <dlfcn.h>

typedef struct { long a, b, c; } Three;                /* 24 bytes */
typedef struct { double x, y, z, w; } Four;            /* 32 bytes */
typedef struct { char tag[100]; } Hundred;
typedef struct { long double v, w; } LD;               /* 32 bytes */

@interface S : NSObject
- (Three)three:(long)a :(long)b :(long)c :(long)d :(long)e;
- (Four)four:(double)a;
- (Hundred)hundred;
- (LD)ld;
- (Three)selfAndCmd;
+ (Three)classThree;
@end
@interface S2 : S
- (Three)selfAndCmd;
@end
@implementation S
- (Three)three:(long)a :(long)b :(long)c :(long)d :(long)e {
    Three t = {a + b, c + d, e};
    return t;
}
- (Four)four:(double)a { Four f = {a, a + 1, a + 2, a + 3}; return f; }
- (Hundred)hundred {
    Hundred h;
    for (int i = 0; i < 100; i++) h.tag[i] = (char)i;
    return h;
}
- (LD)ld { LD l = {2.5L, 3.5L}; return l; }
- (Three)selfAndCmd { Three t = {(long)(self != 0), (long)(_cmd == @selector(selfAndCmd)), 7}; return t; }
+ (Three)classThree { Three t = {1, 2, 3}; return t; }
@end
@implementation S2
- (Three)selfAndCmd { Three t = {100, 200, 300}; return t; }
@end

int test_main(void) {
#if defined(__x86_64__)
    check("objc_msgSend_stret exists", dlsym(RTLD_DEFAULT, "objc_msgSend_stret") != 0, 0);
    id s = [S new], s2 = [S2 new];
    Three t = {0, 0, 0};
    ((void (*)(Three *, id, SEL, long, long, long, long, long))objc_msgSend_stret)(&t, s, sel_registerName("three:::::"), 1, 2, 3, 4, 5);
    check("24-byte struct, five arguments", t.a == 3 && t.b == 7 && t.c == 5, t.a);
    Four f;
    ((void (*)(Four *, id, SEL, double))objc_msgSend_stret)(&f, s, sel_registerName("four:"), 1.5);
    check("32-byte struct of doubles", f.x == 1.5 && f.y == 2.5 && f.z == 3.5 && f.w == 4.5, 0);
    Hundred h;
    ((void (*)(Hundred *, id, SEL))objc_msgSend_stret)(&h, s, sel_registerName("hundred"));
    int ok = 1;
    for (int i = 0; i < 100; i++) if (h.tag[i] != (char)i) ok = 0;
    check("100-byte struct", ok, 0);
    LD l = {0};
    ((void (*)(LD *, id, SEL))objc_msgSend_stret)(&l, s, sel_registerName("ld"));
    check("a struct of two long doubles", l.v == 2.5L && l.w == 3.5L, 0);
    ((void (*)(Three *, id, SEL))objc_msgSend_stret)(&t, s, sel_registerName("selfAndCmd"));
    check("self and _cmd are delivered", t.a == 1 && t.b == 1 && t.c == 7, t.a);
    ((void (*)(Three *, id, SEL))objc_msgSend_stret)(&t, s2, sel_registerName("selfAndCmd"));
    check("override", t.a == 100 && t.c == 300, t.a);
    ((void (*)(Three *, id, SEL))objc_msgSend_stret)(&t, (id)objc_getClass("S"), sel_registerName("classThree"));
    check("class method", t.a == 1 && t.b == 2 && t.c == 3, t.a);
    ((void (*)(Three *, id, SEL))objc_msgSend_stret)(&t, (id)objc_getClass("S2"), sel_registerName("classThree"));
    check("inherited class method", t.a == 1 && t.b == 2 && t.c == 3, t.a);
    /* The buffer is the caller's: neighbours are not written. */
    struct { long before; Three t; long after; } box = {0x1111, {0, 0, 0}, 0x2222};
    ((void (*)(Three *, id, SEL, long, long, long, long, long))objc_msgSend_stret)(&box.t, s, sel_registerName("three:::::"), 9, 9, 9, 9, 9);
    check("the neighbours of the buffer are intact", box.before == 0x1111 && box.after == 0x2222, 0);
    check("and the buffer holds the result", box.t.a == 18 && box.t.b == 18 && box.t.c == 9, box.t.a);
#else
    check("objc_msgSend_stret does not exist on arm64", dlsym(RTLD_DEFAULT, "objc_msgSend_stret") == 0, 0);
    check("objc_msgSendSuper2_stret does not exist on arm64", dlsym(RTLD_DEFAULT, "objc_msgSendSuper2_stret") == 0, 0);
    check("objc_msgSend_fpret does not exist on arm64", dlsym(RTLD_DEFAULT, "objc_msgSend_fpret") == 0, 0);
    /* The struct comes back through x8 with a plain objc_msgSend. */
    id s = [S new];
    Three t = ((Three (*)(id, SEL, long, long, long, long, long))objc_msgSend)(s, sel_registerName("three:::::"), 1, 2, 3, 4, 5);
    check("arm64: objc_msgSend returns the 24-byte struct", t.a == 3 && t.b == 7 && t.c == 5, t.a);
    Hundred h = ((Hundred (*)(id, SEL))objc_msgSend)(s, sel_registerName("hundred"));
    check("arm64: and the 100-byte one", h.tag[99] == 99, 0);
#endif
    return finish();
}
