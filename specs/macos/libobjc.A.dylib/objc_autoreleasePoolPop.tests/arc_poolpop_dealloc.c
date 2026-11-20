// poly-mac: libobjc
// objc_autoreleasePoolPop: an object whose -dealloc autoreleases another object (into the pool being
// popped) has that object released by the same pop; a -dealloc that opens its own pool works.
#include "poly_mobjc.h"
static int inner_dead, outer_dead;
@interface Inner : NSObject @end
@implementation Inner
- (void)dealloc { inner_dead++; [super dealloc]; }
@end
@interface Outer : NSObject @end
@implementation Outer
- (void)dealloc {
    outer_dead++;
    objc_autorelease([Inner new]);
    [super dealloc];
}
@end
@interface Pooled : NSObject @end
static int pooled_inner;
@implementation Pooled
- (void)dealloc {
    void *p = objc_autoreleasePoolPush();
    objc_autorelease([Inner new]);
    objc_autoreleasePoolPop(p);
    pooled_inner = inner_dead;
    [super dealloc];
}
@end
int test_main(void) {
    void *p = objc_autoreleasePoolPush();
    objc_autorelease([Outer new]);
    objc_autoreleasePoolPop(p);
    value("outer deallocated", outer_dead);
    value("the object it autoreleased while being popped is gone too", inner_dead);
    inner_dead = 0;
    p = objc_autoreleasePoolPush();
    objc_autorelease([Pooled new]);
    objc_autoreleasePoolPop(p);
    value("a pool opened inside -dealloc drained inside it", pooled_inner);
    return finish();
}
