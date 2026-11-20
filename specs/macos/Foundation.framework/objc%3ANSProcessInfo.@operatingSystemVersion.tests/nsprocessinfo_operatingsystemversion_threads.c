/* poly-mac: foundation */
/* The version read on another thread is the same; the struct can be passed by value to
 * -isOperatingSystemAtLeastVersion: on that thread; the version has major at least 13 there
 * too. */
#include "poly_mfn.h"

static volatile int done;
static NSOperatingSystemVersion from_thread;
static int at_least;

int test_main(void) {
    NSOperatingSystemVersion v = [NSProcessInfo processInfo].operatingSystemVersion;
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            from_thread = [NSProcessInfo processInfo].operatingSystemVersion;
            at_least = [[NSProcessInfo processInfo] isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion){13, 0, 0}];
        }
        done = 1;
    }];
    for (int i = 0; i < 500 && !done; i++) [NSThread sleepForTimeInterval:0.01];
    CK(done && at_least);
    CK(from_thread.majorVersion == v.majorVersion && from_thread.minorVersion == v.minorVersion && from_thread.patchVersion == v.patchVersion);
    CK(from_thread.majorVersion >= 13);
    return finish();
}
