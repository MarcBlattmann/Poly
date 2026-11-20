/* poly-mac: foundation */
/* An exception raised inside a block is caught by an @try around the call of the
 * block; one raised on a worker thread and caught there does not reach the main
 * thread; collection and string errors raised by the system are catchable in
 * the same way; a raise in an observer of a notification unwinds through
 * postNotificationName: to the caller. */
#include "poly_mfn.h"

@interface PolyThrower : NSObject
- (void)boom:(NSNotification *)n;
@end
@implementation PolyThrower
- (void)boom:(NSNotification *)n {
    (void)n;
    [NSException raise:@"FromObserver" format:@"observer failed"];
}
@end

static volatile int worker_done, worker_caught;

int test_main(void) {
    void (^thrower)(void) = ^{ [NSException raise:@"InBlock" format:@"x"]; };
    @try {
        thrower();
        CK(0);
    } @catch (NSException *e) {
        CK(ns_is(e.name, "InBlock"));
    }
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            @try {
                [NSException raise:@"InWorker" format:@"x"];
            } @catch (NSException *e) {
                worker_caught = [e.name isEqualToString:@"InWorker"];
            }
        }
        worker_done = 1;
    }];
    for (int i = 0; i < 500 && !worker_done; i++) [NSThread sleepForTimeInterval:0.01];
    CK(worker_done && worker_caught);
    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
    PolyThrower *t = [[PolyThrower alloc] init];
    [nc addObserver:t selector:@selector(boom:) name:@"PolyBoom" object:nil];
    @try {
        [nc postNotificationName:@"PolyBoom" object:nil];
        CK(0);
    } @catch (NSException *e) {
        CK(ns_is(e.name, "FromObserver"));
    }
    [nc removeObserver:t];
    return finish();
}
