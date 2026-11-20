/* poly-mac: foundation */
/* In a copy running as T.app/Contents/MacOS/t the process name is 't' (the
 * file name of the executable, not the bundle), processIdentifier is the pid
 * the kernel reports for getpid (raw system call 20), the same object is
 * answered on another thread, and the arguments carry the program's own path. */
#include "poly_mfn.h"

static volatile int done;
static NSProcessInfo *from_thread;

int test_main(void) {
    if (!cf_enter_app(CF_APP_PLIST)) return 1;
    NSProcessInfo *pi = [NSProcessInfo processInfo];
    CK(ns_is(pi.processName, "t"));
    CK(pi.processIdentifier == (int)bsd3(20, 0, 0, 0).ret);
    CK(ns_is(pi.arguments[0].lastPathComponent, "t"));
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            from_thread = [NSProcessInfo processInfo];
        }
        done = 1;
    }];
    for (int i = 0; i < 500 && !done; i++) [NSThread sleepForTimeInterval:0.01];
    CK(done && from_thread == pi);
    return finish();
}
