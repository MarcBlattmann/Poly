/* poly-mac: foundation */
/* detachNewThreadSelector:toTarget:withObject: starts a thread that sends the
 * selector to the target with the object: it runs concurrently (the caller's
 * call returns first), on a thread that is not the main one, with the object
 * as argument, and the target and object are retained for it; several threads
 * run to completion. */
#include "poly_mfn.h"

@interface PolyWorker : NSObject {
@public
    volatile int ran;
    int onMain;
    NSString *got;
}
- (void)work:(id)arg;
@end

@implementation PolyWorker
- (void)work:(id)arg {
    @autoreleasepool {
        onMain = [NSThread isMainThread];
        got = arg;
        ran++;
    }
}
@end

int test_main(void) {
    PolyWorker *w = [[PolyWorker alloc] init];
    [NSThread detachNewThreadSelector:@selector(work:) toTarget:w withObject:ns_make("argument")];
    for (int i = 0; i < 500 && !w->ran; i++) [NSThread sleepForTimeInterval:0.01];
    CK(w->ran == 1);
    CK(w->onMain == 0);
    CK(ns_is(w->got, "argument"));
    PolyWorker *many[5];
    for (int i = 0; i < 5; i++) {
        many[i] = [[PolyWorker alloc] init];
        [NSThread detachNewThreadSelector:@selector(work:) toTarget:many[i] withObject:nil];
    }
    for (int i = 0; i < 500; i++) {
        int all = 1;
        for (int j = 0; j < 5; j++) all &= many[j]->ran == 1;
        if (all) break;
        [NSThread sleepForTimeInterval:0.01];
    }
    for (int j = 0; j < 5; j++) CK(many[j]->ran == 1 && many[j]->onMain == 0);
    return finish();
}
