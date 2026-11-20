/* poly-mac: foundation */
/* The default manager is the same object on a worker thread; a file made on one
 * thread is visible on the other; the listing of a directory with several
 * files has every name once (sorted before comparing, the order is not specified). */
#include "poly_mfn.h"

static volatile int done;
static NSFileManager *from_thread;
static int saw_file;

int test_main(void) {
    NSFileManager *fm = [NSFileManager defaultManager];
    [fm createFileAtPath:@"main-made" contents:nil attributes:nil];
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            from_thread = [NSFileManager defaultManager];
            saw_file = [from_thread fileExistsAtPath:@"main-made"];
            [from_thread createFileAtPath:@"worker-made" contents:nil attributes:nil];
        }
        done = 1;
    }];
    for (int i = 0; i < 500 && !done; i++) [NSThread sleepForTimeInterval:0.01];
    CK(done && from_thread == fm && saw_file);
    CK([fm fileExistsAtPath:@"worker-made"]);
    for (int i = 0; i < 5; i++) [fm createFileAtPath:[NSString stringWithFormat:@"f%d", i] contents:nil attributes:nil];
    NSArray *list = [[fm contentsOfDirectoryAtPath:@"." error:nil] sortedArrayUsingSelector:@selector(compare:)];
    CK(ns_is([list componentsJoinedByString:@","], "f0,f1,f2,f3,f4,main-made,worker-made"));
    return finish();
}
