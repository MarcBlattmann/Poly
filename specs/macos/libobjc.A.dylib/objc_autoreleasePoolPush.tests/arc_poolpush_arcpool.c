// poly-mac: libobjc
// poly-mac: arc
// objc_autoreleasePoolPush and objc_autoreleasePoolPop as ARC-compiled code uses them: an
// @autoreleasepool block pushes at entry and pops at exit (also by break and by return from a loop
// body); objects autoreleased inside die at the block's end, nested blocks end first.
#include "poly_mobjc.h"
static int deallocs;
@interface Obj : NSObject @end
@implementation Obj
- (void)dealloc { deallocs++; }
@end
// Puts one autoreleased object into the innermost pool.
__attribute__((noinline)) static void give(__autoreleasing Obj **out) { *out = [Obj new]; }
__attribute__((noinline)) static void give_many(int n) {
    for (int i = 0; i < n; i++) { Obj *__autoreleasing t; give(&t); }
}
int test_main(void) {
    @autoreleasepool {
        give_many(10);
        value("deallocs inside the block", deallocs);
        @autoreleasepool {
            give_many(5);
            value("deallocs inside the inner block", deallocs);
        }
        value("deallocs after the inner block", deallocs);
    }
    value("deallocs after the outer block", deallocs);
    deallocs = 0;
    for (int i = 0; i < 3; i++) {
        @autoreleasepool {
            give_many(4);
            if (i == 1) break;
        }
        value("deallocs after a loop iteration", deallocs);
    }
    value("deallocs after the break", deallocs);
    // the pool stack is back where it was: a plain push/pop pair still works
    void *p = objc_autoreleasePoolPush();
    give_many(2);
    objc_autoreleasePoolPop(p);
    value("deallocs after a manual pool", deallocs);
    return finish();
}
