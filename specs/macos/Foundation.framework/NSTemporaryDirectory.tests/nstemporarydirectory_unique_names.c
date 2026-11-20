/* poly-mac: foundation */
/* Names made under the temporary directory with NSUUID or globallyUniqueString are
 * all different and can be created, listed and removed there; the directory
 * is not the current directory and is not the home directory. */
#include "poly_mfn.h"

int test_main(void) {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSString *tmp = NSTemporaryDirectory();
    NSMutableSet *names = [NSMutableSet set];
    NSMutableArray *paths = [NSMutableArray array];
    for (int i = 0; i < 5; i++) {
        NSString *n = [NSString stringWithFormat:@"poly-nsspec-%@", [[NSUUID UUID] UUIDString]];
        [names addObject:n];
        NSString *p = [tmp stringByAppendingPathComponent:n];
        CK([fm createDirectoryAtPath:p withIntermediateDirectories:NO attributes:nil error:nil]);
        [paths addObject:p];
    }
    CK(names.count == 5);
    for (NSString *p in paths) CK([fm fileExistsAtPath:p]);
    for (NSString *p in paths) [fm removeItemAtPath:p error:nil];
    for (NSString *p in paths) CK(![fm fileExistsAtPath:p]);
    CK(![tmp isEqualToString:fm.currentDirectoryPath] && ![tmp hasPrefix:[NSHomeDirectory() stringByAppendingString:@"/Documents"]]);
    return finish();
}
