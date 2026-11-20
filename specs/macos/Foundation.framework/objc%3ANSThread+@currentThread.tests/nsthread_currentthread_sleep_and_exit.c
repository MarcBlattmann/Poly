/* poly-mac: foundation */
/* sleepForTimeInterval: blocks the calling thread for about the time asked
 * (at least 0.09 s of 0.1 s, well under 2 s) and zero returns at once; a worker
 * that calls +exit ends its thread without running the rest of its block; the
 * thread's isFinished becomes YES afterwards. */
#include "poly_mfn.h"

static volatile int before_exit, after_exit;
static NSThread *worker;

int test_main(void) {
    NSDate *t0 = [NSDate date];
    [NSThread sleepForTimeInterval:0.1];
    NSTimeInterval d = [[NSDate date] timeIntervalSinceDate:t0];
    CK(d >= 0.09 && d < 2.0);
    t0 = [NSDate date];
    [NSThread sleepForTimeInterval:0];
    CK([[NSDate date] timeIntervalSinceDate:t0] < 0.05);
    NSThread *t = [[NSThread alloc] initWithBlock:^{
        before_exit = 1;
        [NSThread exit];
        after_exit = 1;
    }];
    worker = t;
    [t start];
    for (int i = 0; i < 500 && !t.isFinished; i++) [NSThread sleepForTimeInterval:0.01];
    CK(before_exit && !after_exit);
    CK(t.isFinished && !t.isExecuting);
    return finish();
}
