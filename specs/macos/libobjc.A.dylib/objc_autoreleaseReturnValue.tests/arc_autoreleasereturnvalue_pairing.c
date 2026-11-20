// poly-mac: libobjc
// objc_autoreleaseReturnValue paired with objc_retainAutoreleasedReturnValue by hand: the caller's
// code right after the call is what the runtime looks at (on arm64 a marker instruction, on x86_64
// the call of the retaining function); a paired call leaves the retain count unchanged and the
// pool empty.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
__attribute__((noinline)) static id ar(id o) { return objc_autoreleaseReturnValue(o); }
#if defined(__arm64__)
#define MARK() __asm__ volatile("mov x29, x29")
#else
#define MARK() ((void)0)
#endif
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    id o = [Obj new];
    id r = ar(o); MARK(); r = objc_retainAutoreleasedReturnValue(r);
    check("same object", r == o, 0);
    value("paired: retainCount", (i64)retain_count(r));
    objc_autoreleasePoolPop(p);
    value("paired: deallocs after the pop (nothing was in the pool)", deallocs);
    objc_release(r);
    value("paired: deallocs after the release", deallocs);
    // unpaired: the object is in the pool
    deallocs = 0;
    p = objc_autoreleasePoolPush();
    o = [Obj new];
    r = ar(o);
    value("unpaired: retainCount", (i64)retain_count(r));
    objc_autoreleasePoolPop(p);
    value("unpaired: deallocs after the pop", deallocs);
    return finish();
}
