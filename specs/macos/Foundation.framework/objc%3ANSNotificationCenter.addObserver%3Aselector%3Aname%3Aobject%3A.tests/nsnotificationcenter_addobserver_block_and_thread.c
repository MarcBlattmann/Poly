/* poly-mac: foundation */
/* The block form addObserverForName:object:queue:usingBlock: calls the block
 * before post returns when the queue is nil (on the posting thread) and returns a
 * token that removeObserver: takes; a post from another thread reaches an
 * observer on that thread; a notification made by hand and posted with
 * postNotification: carries its name, object and info. */
#include "poly_mfn.h"

static volatile int posted_from_worker;

int test_main(void) {
    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
    __block int hits = 0;
    __block int onMain = -1;
    id tok = [nc addObserverForName:@"PolyBlock" object:nil queue:nil usingBlock:^(NSNotification *n) {
        (void)n;
        hits++;
        onMain = [NSThread isMainThread];
    }];
    [nc postNotificationName:@"PolyBlock" object:nil];
    CK(hits == 1 && onMain == 1);
    [nc removeObserver:tok];
    [nc postNotificationName:@"PolyBlock" object:nil];
    CK(hits == 1);
    id tok2 = [nc addObserverForName:@"PolyThread" object:nil queue:nil usingBlock:^(NSNotification *n) {
        (void)n;
        hits += 10;
        onMain = [NSThread isMainThread];
    }];
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            [[NSNotificationCenter defaultCenter] postNotificationName:@"PolyThread" object:nil];
        }
        posted_from_worker = 1;
    }];
    for (int i = 0; i < 500 && !posted_from_worker; i++) [NSThread sleepForTimeInterval:0.01];
    CK(posted_from_worker && hits == 11 && onMain == 0);
    [nc removeObserver:tok2];
    __block NSString *seen = nil;
    id tok3 = [nc addObserverForName:@"PolyHand" object:nil queue:nil usingBlock:^(NSNotification *n) { seen = n.name; }];
    [nc postNotification:[NSNotification notificationWithName:@"PolyHand" object:nil userInfo:@{@"k": @"v"}]];
    CK(ns_is(seen, "PolyHand"));
    [nc removeObserver:tok3];
    return finish();
}
