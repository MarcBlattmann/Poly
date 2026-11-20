// poly-mac: libobjc
// objc_retainAutoreleasedReturnValue called after a call that returned through
// objc_autoreleaseReturnValue: many in a row, nil results, and the handshake does not leak
// into a later, unrelated call.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
#if defined(__arm64__)
#define MARK() __asm__ volatile("mov x29, x29")
#else
#define MARK() ((void)0)
#endif
__attribute__((noinline)) static id make(void) { return objc_autoreleaseReturnValue([Obj new]); }
__attribute__((noinline)) static id make_or_nil(int want) { return objc_autoreleaseReturnValue(want ? [Obj new] : nil); }
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    for (int i = 0; i < 100; i++) {
        id x = make(); MARK(); x = objc_retainAutoreleasedReturnValue(x);
        objc_release(x);
    }
    value("deallocs after 100 scopes, pool not yet popped", deallocs);
    id a = make_or_nil(0); MARK(); a = objc_retainAutoreleasedReturnValue(a);
    check("nil result", a == nil, 0);
    id b = make_or_nil(1); MARK(); b = objc_retainAutoreleasedReturnValue(b);
    check("non-nil result", b != nil, 0);
    value("retainCount of the second result", (i64)retain_count(b));
    // an unrelated object passed to objc_retainAutoreleasedReturnValue later is simply retained
    id c = [Obj new];
    id c2 = objc_retainAutoreleasedReturnValue(c);
    value("retainCount of an unrelated object", (i64)retain_count(c2));
    objc_release(c); objc_release(c2); objc_release(b);
    objc_autoreleasePoolPop(p);
    value("deallocs after the pop", deallocs);
    return finish();
}
