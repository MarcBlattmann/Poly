/* poly-mac: foundation */
/* NSHomeDirectory agrees with the other ways to ask: the file manager's
 * homeDirectoryForCurrentUser, the user domain search paths (their common prefix),
 * the expansion of '~', and NSHomeDirectoryForUser of the current user name. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *home = NSHomeDirectory();
    CK([[NSFileManager defaultManager].homeDirectoryForCurrentUser.path isEqualToString:home]);
    CK([NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSUserDomainMask, YES).firstObject hasPrefix:home]);
    CK([NSHomeDirectoryForUser(NSUserName()) isEqualToString:home]);
    CK([NSFullUserName() length] > 0);
    CK([@"~" stringByExpandingTildeInPath].length == home.length);
    CK([[@"~/Library" stringByExpandingTildeInPath] isEqualToString:[home stringByAppendingPathComponent:@"Library"]]);
    CK([[home stringByAbbreviatingWithTildeInPath] isEqualToString:@"~"]);
    return finish();
}
