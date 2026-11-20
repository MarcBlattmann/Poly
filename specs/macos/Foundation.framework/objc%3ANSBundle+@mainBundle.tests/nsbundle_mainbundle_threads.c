/* poly-mac: foundation */
/* The main bundle is the same object on a worker thread; its path facts from there are
 * the same strings as on the main thread; reading the info dictionary from two
 * threads at once gives equal results. */
#include "poly_mfn.h"

static volatile int done;
static NSBundle *from_thread;
static NSString *path_from_thread;

int test_main(void) {
    NSBundle *b = [NSBundle mainBundle];
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            from_thread = [NSBundle mainBundle];
            path_from_thread = [from_thread.bundlePath copy];
        }
        done = 1;
    }];
    for (int i = 0; i < 500 && !done; i++) [NSThread sleepForTimeInterval:0.01];
    CK(done && from_thread == b);
    CK([path_from_thread isEqualToString:b.bundlePath]);
    CK([from_thread.infoDictionary isEqualToDictionary:b.infoDictionary]);
    return finish();
}
