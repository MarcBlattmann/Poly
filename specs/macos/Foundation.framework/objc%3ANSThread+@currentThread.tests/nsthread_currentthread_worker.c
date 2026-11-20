/* poly-mac: foundation */
/* On a thread made with detachNewThreadWithBlock: currentThread is another
 * object, not the main thread, stable inside the thread, with its own empty thread
 * dictionary; the main thread sees its own dictionary unchanged. The thread's
 * facts are posted back through plain variables read after the thread
 * flagged it was done. */
#include "poly_mfn.h"

static volatile int done;
static int worker_is_main = -1, worker_same = -1, worker_dict_empty = -1;
static NSThread *worker_thread;

int test_main(void) {
    NSThread *mainThread = [NSThread currentThread];
    mainThread.threadDictionary[@"who"] = @"main";
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            NSThread *t = [NSThread currentThread];
            worker_thread = t;
            worker_is_main = [NSThread isMainThread];
            worker_same = t == [NSThread currentThread];
            worker_dict_empty = t.threadDictionary[@"who"] == nil;
            t.threadDictionary[@"who"] = @"worker";
        }
        done = 1;
    }];
    for (int i = 0; i < 500 && !done; i++) [NSThread sleepForTimeInterval:0.01];
    CK(done);
    CK(worker_is_main == 0 && worker_same == 1 && worker_dict_empty == 1);
    CK(worker_thread != mainThread && worker_thread != nil);
    CK(ns_is(mainThread.threadDictionary[@"who"], "main"));
    CK([NSThread isMainThread]);
    return finish();
}
