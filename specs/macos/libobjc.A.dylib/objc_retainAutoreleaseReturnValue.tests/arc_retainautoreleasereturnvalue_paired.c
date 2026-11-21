// poly-mac: libobjc
// objc_retainAutoreleaseReturnValue paired by hand with objc_retainAutoreleasedReturnValue:
// the paired call adds one count and leaves nothing in the pool.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; [super dealloc]; }
@end
__attribute__((noinline)) static id rar(id o) { return objc_retainAutoreleaseReturnValue(o); }
#if defined(__arm64__)
#define MARK() __asm__ volatile("mov x29, x29")
#else
#define MARK() ((void)0)
#endif
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    id o = [Obj new];
    id r = rar(o); MARK(); r = objc_retainAutoreleasedReturnValue(r);
    value("paired: retainCount", (i64)retain_count(r));
    objc_autoreleasePoolPop(p);
    value("paired: retainCount after the pop", (i64)retain_count(r));
    objc_release(r);
    value("paired: deallocs after one release", deallocs);
    objc_release(r);
    value("paired: deallocs after two releases", deallocs);
    return finish();
}
