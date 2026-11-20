/* poly-mac: foundation */
/* Each detached thread has its own NSThread object and its own thread dictionary; ten
 * threads started in a row report ten different threads, all not the main
 * one, and the main thread's dictionary is untouched. */
#include "poly_mfn.h"

@interface PolyReporter : NSObject {
@public
    volatile int done;
    NSThread *thread;
}
- (void)report:(id)unused;
@end
@implementation PolyReporter
- (void)report:(id)unused {
    @autoreleasepool {
        thread = [NSThread currentThread];
        thread.threadDictionary[@"mine"] = @"yes";
        done = 1;
    }
}
@end

int test_main(void) {
    NSMutableArray *reps = [NSMutableArray array];
    for (int i = 0; i < 10; i++) {
        PolyReporter *r = [[PolyReporter alloc] init];
        [reps addObject:r];
        [NSThread detachNewThreadSelector:@selector(report:) toTarget:r withObject:nil];
    }
    for (int i = 0; i < 500; i++) {
        int all = 1;
        for (PolyReporter *r in reps) all &= r->done;
        if (all) break;
        [NSThread sleepForTimeInterval:0.01];
    }
    NSMutableSet *seen = [NSMutableSet set];
    for (PolyReporter *r in reps) {
        CK(r->done && r->thread != nil && r->thread != [NSThread mainThread]);
        [seen addObject:[NSValue valueWithNonretainedObject:r->thread]];
    }
    CK(seen.count == 10);
    CK([NSThread currentThread].threadDictionary[@"mine"] == nil);
    return finish();
}
