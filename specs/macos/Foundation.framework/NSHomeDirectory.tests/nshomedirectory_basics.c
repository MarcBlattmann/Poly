/* poly-mac: foundation */
/* NSHomeDirectory is an absolute path without trailing slash to an existing
 * directory, the same on every call, from the user database (the HOME
 * variable is not consulted: the program starts with an empty environment and
 * gets the same answer); NSUserName is not empty and the home directory's last
 * component is the user name for an ordinary account; the tilde expands to it. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *home = NSHomeDirectory();
    BOOL isDir = NO;
    CK(home != nil && [home hasPrefix:@"/"] && ![home hasSuffix:@"/"] && home.length > 1);
    CK([[NSFileManager defaultManager] fileExistsAtPath:home isDirectory:&isDir] && isDir);
    CK([NSHomeDirectory() isEqualToString:home]);
    CK(NSUserName().length > 0);
    CK([home hasSuffix:[@"/" stringByAppendingString:NSUserName()]]);
    CK([[@"~" stringByExpandingTildeInPath] isEqualToString:home]);
    CK([[@"~/x" stringByExpandingTildeInPath] isEqualToString:[home stringByAppendingPathComponent:@"x"]]);
    CK(ns_is([NSHomeDirectory() class] == [NSHomeDirectory() class] ? @"same" : @"diff", "same"));
    return finish();
}
