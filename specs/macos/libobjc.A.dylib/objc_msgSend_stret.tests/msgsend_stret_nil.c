// poly-mac: libobjc
// objc_msgSend_stret (x86_64) with a nil receiver: the result buffer is left
// exactly as it was (it is not zeroed), no method runs, and +resolve...
// is not asked. On arm64 the same message through objc_msgSend with a
// struct return leaves memory alone as well.
#include "poly_mobjc.h"

typedef struct { long a, b, c, d; } Big;
static int ran;
@interface N : NSObject
- (Big)big;
@end
@implementation N
- (Big)big { ran++; Big b = {1, 2, 3, 4}; return b; }
+ (BOOL)resolveInstanceMethod:(SEL)s { (void)s; ran += 100; return NO; }
@end

static void poison(Big *b) { for (unsigned i = 0; i < sizeof *b; i++) ((unsigned char *)b)[i] = 0x5a; }
static int untouched(const Big *b) {
    for (unsigned i = 0; i < sizeof *b; i++) if (((const unsigned char *)b)[i] != 0x5a) return 0;
    return 1;
}

int test_main(void) {
#if defined(__x86_64__)
    Big b;
    poison(&b);
    ((void (*)(Big *, id, SEL))objc_msgSend_stret)(&b, (id)0, sel_registerName("big"));
    check("nil receiver leaves the buffer alone", untouched(&b), 0);
    poison(&b);
    ((void (*)(Big *, Class, SEL))objc_msgSend_stret)(&b, (Class)0, sel_registerName("big"));
    check("nil Class too", untouched(&b), 0);
    poison(&b);
    ((void (*)(Big *, id, SEL))objc_msgSend_stret)(&b, (id)0, sel_registerName("neverRegisteredSelectorForStret"));
    check("an unknown selector too", untouched(&b), 0);
    check("nothing ran", ran == 0, ran);
    id n = [N new];
    ((void (*)(Big *, id, SEL))objc_msgSend_stret)(&b, n, sel_registerName("big"));
    check("a live receiver fills the buffer", b.a == 1 && b.d == 4, b.a);
    check("and the method ran once", ran == 1, ran);
#else
    check("x86_64 only (objc_msgSend_stret does not exist on arm64)", 1, 0);
#endif
    return finish();
}
