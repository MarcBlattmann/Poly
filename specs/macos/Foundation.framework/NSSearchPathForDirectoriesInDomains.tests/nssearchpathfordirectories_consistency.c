/* poly-mac: foundation */
/* The search paths agree with the file manager's URLsForDirectory:inDomains: (same
 * paths, as file URLs), repeated calls give equal arrays, expandTilde YES and NO
 * differ only in the home prefix, and the array is a CFArray of CFStrings. */
#include "poly_mfn.h"

int test_main(void) {
    NSArray *p = NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSUserDomainMask, YES);
    NSArray *urls = [[NSFileManager defaultManager] URLsForDirectory:NSLibraryDirectory inDomains:NSUserDomainMask];
    CK(p.count == 1 && urls.count == 1 && [p[0] isEqualToString:[urls[0] path]]);
    CK([p isEqualToArray:NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSUserDomainMask, YES)]);
    NSArray *t = NSSearchPathForDirectoriesInDomains(NSLibraryDirectory, NSUserDomainMask, NO);
    CK([[t[0] stringByExpandingTildeInPath] isEqualToString:p[0]]);
    CK(CFGetTypeID((__bridge CFTypeRef)p) == CFArrayGetTypeID() && CFGetTypeID((__bridge CFTypeRef)p[0]) == CFStringGetTypeID());
    CK([NSSearchPathForDirectoriesInDomains(NSDocumentDirectory, NSUserDomainMask, YES)[0] isEqualToString:[NSHomeDirectory() stringByAppendingPathComponent:@"Documents"]]);
    return finish();
}
