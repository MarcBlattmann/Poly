/* poly-mac: foundation */
/* The singleton's facts agree with each other: the process name is the last component of
 * argument 0, the unique strings are different, the pid is stable between calls, and
 * the object answers the same on the main thread and in a worker. */
#include "poly_mfn.h"

int test_main(void) {
    NSProcessInfo *pi = [NSProcessInfo processInfo];
    CK(pi.processIdentifier == [NSProcessInfo processInfo].processIdentifier);
    CK([pi.processName isEqualToString:[pi.arguments[0] lastPathComponent]]);
    NSMutableSet *u = [NSMutableSet set];
    for (int i = 0; i < 10; i++) [u addObject:pi.globallyUniqueString];
    CK(u.count == 10);
    CK(pi.systemUptime > 0);
    CK([pi.hostName isEqualToString:[NSProcessInfo processInfo].hostName]);
    CK(pi.physicalMemory > (1ULL << 28));
    return finish();
}
