// poly-mac: libobjc
// objc_msgSend to a nil receiver: nothing is called, and the result
// registers read as zero (integer, pointer, floating point and the
// two-register struct returns). Memory returns are not written.
#include "poly_mobjc.h"

typedef struct { long a, b; } Two;
typedef struct { int a, b; } TwoInts;
typedef struct { double x, y; } DD;
typedef struct { double x, y, z, w; } D4;
typedef struct { float x, y, z, w; } F4;
typedef struct { long a, b, c, d; } Big;

static int called;
static int resolver_called;

@interface Probe : NSObject
- (int)tag;
+ (BOOL)resolveInstanceMethod:(SEL)s;
@end
@implementation Probe
- (int)tag { called++; return 1; }
+ (BOOL)resolveInstanceMethod:(SEL)s { (void)s; resolver_called++; return NO; }
@end

static int arg_evaluated;
static long side(long v) { arg_evaluated++; return v; }

int test_main(void) {
    id nilobj = (id)0;
    SEL tag = sel_registerName("tag");

    check("int", ((int (*)(id, SEL))objc_msgSend)(nilobj, tag) == 0, 0);
    check("long", ((long (*)(id, SEL))objc_msgSend)(nilobj, tag) == 0, 0);
    check("char", ((char (*)(id, SEL))objc_msgSend)(nilobj, tag) == 0, 0);
    check("BOOL", ((BOOL (*)(id, SEL))objc_msgSend)(nilobj, tag) == NO, 0);
    check("pointer", ((void *(*)(id, SEL))objc_msgSend)(nilobj, tag) == 0, 0);
    check("id", ((id (*)(id, SEL))objc_msgSend)(nilobj, tag) == nilobj, 0);
    check("double", ((double (*)(id, SEL))objc_msgSend)(nilobj, tag) == 0.0, 0);
    check("float", ((float (*)(id, SEL))objc_msgSend)(nilobj, tag) == 0.0f, 0);
    {
        Two t = {1, 1};
        t = ((Two (*)(id, SEL))objc_msgSend)(nilobj, tag);
        check("two-long struct (both registers zero)", t.a == 0 && t.b == 0, t.a);
        TwoInts ti = {1, 1};
        ti = ((TwoInts (*)(id, SEL))objc_msgSend)(nilobj, tag);
        check("two-int struct", ti.a == 0 && ti.b == 0, ti.a);
        DD dd = {1, 1};
        dd = ((DD (*)(id, SEL))objc_msgSend)(nilobj, tag);
        check("two-double struct (both floating registers zero)", dd.x == 0.0 && dd.y == 0.0, 0);
    }
#if defined(__arm64__)
    {
        F4 f = {1, 1, 1, 1};
        f = ((F4 (*)(id, SEL))objc_msgSend)(nilobj, tag);
        check("four-float struct (arm64 registers s0-s3 zero)", f.x == 0 && f.y == 0 && f.z == 0 && f.w == 0, 0);
        D4 d4 = {1, 1, 1, 1};
        d4 = ((D4 (*)(id, SEL))objc_msgSend)(nilobj, tag);
        check("four-double struct (arm64 registers d0-d3 zero)", d4.x == 0 && d4.y == 0 && d4.z == 0 && d4.w == 0, 0);
    }
#endif
    check("no method was called", called == 0, called);
    check("+resolveInstanceMethod: is not asked", resolver_called == 0, resolver_called);

    /* The arguments are evaluated by the caller, as for any call. */
    ((void (*)(id, SEL, long))objc_msgSend)(nilobj, sel_registerName("setValue:"), side(5));
    check("arguments are evaluated", arg_evaluated == 1, arg_evaluated);

    /* A selector nobody implements or registered before is no problem either. */
    check("unknown selector", ((long (*)(id, SEL))objc_msgSend)(nilobj, sel_registerName("neverImplementedSelectorName")) == 0, 0);
    check("NULL selector", ((long (*)(id, SEL))objc_msgSend)(nilobj, (SEL)0) == 0, 0);
    check("nil Class", ((long (*)(Class, SEL))objc_msgSend)((Class)0, tag) == 0, 0);

#if defined(__x86_64__)
    /* x86_64: a struct returned in memory (objc_msgSend_stret) is not touched. */
    {
        Big b;
        for (int i = 0; i < (int)sizeof b; i++) ((unsigned char *)&b)[i] = 0x55;
        ((void (*)(Big *, id, SEL))objc_msgSend_stret)(&b, nilobj, tag);
        check("stret leaves the buffer alone", b.a == 0x5555555555555555 && b.d == 0x5555555555555555, 0);
        long double ld = ((long double (*)(id, SEL))objc_msgSend_fpret)(nilobj, tag);
        check("fpret long double reads zero", ld == 0.0L, 0);
    }
#endif
    /* A live object is still reached afterwards. */
    id p = [Probe new];
    check("a live receiver works", ((int (*)(id, SEL))objc_msgSend)(p, tag) == 1, 0);
    check("and the method ran once", called == 1, called);
    return finish();
}
