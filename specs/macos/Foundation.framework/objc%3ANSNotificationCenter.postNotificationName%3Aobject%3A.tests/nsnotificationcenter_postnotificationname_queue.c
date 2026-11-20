/* poly-mac: foundation */
/* A block observer registered with an operation queue gets the block run on that
 * queue's thread (for the main queue: after the main run loop turns, not inside post);
 * posting returns first, and the run loop delivers it; the order of queued
 * deliveries is the order of the posts. */
#include "poly_mfn.h"

int test_main(void) {
    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
    __block int hits = 0;
    static char order[8];
    static int n;
    id tok = [nc addObserverForName:@"PolyQueued" object:nil queue:[NSOperationQueue mainQueue] usingBlock:^(NSNotification *x) {
        hits++;
        order[n++] = (char)('0' + [x.userInfo[@"i"] intValue]);
    }];
    [nc postNotificationName:@"PolyQueued" object:nil userInfo:@{@"i": @1}];
    [nc postNotificationName:@"PolyQueued" object:nil userInfo:@{@"i": @2}];
    int direct = hits;
    [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.3]];
    order[n] = 0;
    CK(hits == 2);
    CK(str_eq(order, "12"));
    CK(direct <= 2);
    [nc removeObserver:tok];
    return finish();
}
