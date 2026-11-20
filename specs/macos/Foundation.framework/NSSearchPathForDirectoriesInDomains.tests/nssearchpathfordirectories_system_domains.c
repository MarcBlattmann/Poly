/* poly-mac: foundation */
/* The local domain gives /Applications and /Library, the all-domains mask lists the user's
 * Library first and the system ones after it (the user, local, network and
 * system Library directories), the result is a plain array of strings that
 * contains no duplicates for a single domain. */
#include "poly_mfn.h"

int test_main(void) {
    NSArray *apps = NSSearchPathForDirectoriesInDomains(NSApplicationDirectory, NSLocalDomainMask, YES);
    CK(apps.count == 1 && ns_is(apps[0], "/Applications"));
    NSArray *lib = NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSLocalDomainMask, YES);
    CK(lib.count == 1 && ns_is(lib[0], "/Library"));
    NSArray *all = NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSAllDomainsMask, YES);
    CK(all.count >= 4);
    CK([all[0] hasPrefix:NSHomeDirectory()] && [all[0] hasSuffix:@"/Library"]);
    CK([all containsObject:@"/Library"] && [all containsObject:@"/System/Library"] && [all containsObject:@"/Network/Library"]);
    for (id p in all) CK([p isKindOfClass:[NSString class]] && [p hasPrefix:@"/"]);
    CK(CFGetTypeID((__bridge CFTypeRef)all) == CFArrayGetTypeID());
    NSArray *user = NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSUserDomainMask, YES);
    CK(user.count == 1 && [user[0] isEqualToString:all[0]]);
    return finish();
}
