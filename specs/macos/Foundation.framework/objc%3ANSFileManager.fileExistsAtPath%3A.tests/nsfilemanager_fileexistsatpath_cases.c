/* poly-mac: foundation */
/* fileExistsAtPath: is true for existing files and directories (absolute or
 * relative to the current directory), false for a missing one, for the empty
 * string and for '~' (no tilde expansion); the isDirectory: form sets the flag; a
 * symbolic link is followed. */
#include "poly_mfn.h"

int test_main(void) {
    NSFileManager *fm = [NSFileManager defaultManager];
    BOOL isDir = NO;
    CK([fm fileExistsAtPath:@"/"]);
    CK([fm fileExistsAtPath:@"/" isDirectory:&isDir] && isDir);
    CK(![fm fileExistsAtPath:@"/nonexistent-poly-path"]);
    CK([fm fileExistsAtPath:@"."] && [fm fileExistsAtPath:@"./"]);
    CK(![fm fileExistsAtPath:@""]);
    CK(![fm fileExistsAtPath:@"~"]);
    [fm createFileAtPath:@"f" contents:nil attributes:nil];
    isDir = YES;
    CK([fm fileExistsAtPath:@"f" isDirectory:&isDir] && !isDir);
    [fm createDirectoryAtPath:@"d" withIntermediateDirectories:NO attributes:nil error:nil];
    CK([fm fileExistsAtPath:@"d" isDirectory:&isDir] && isDir);
    CK([fm createSymbolicLinkAtPath:@"ln" withDestinationPath:@"d" error:nil]);
    isDir = NO;
    CK([fm fileExistsAtPath:@"ln" isDirectory:&isDir] && isDir);
    [fm createSymbolicLinkAtPath:@"dangling" withDestinationPath:@"nowhere" error:nil];
    CK(![fm fileExistsAtPath:@"dangling"]);
    CK([fm fileExistsAtPath:[[fm currentDirectoryPath] stringByAppendingPathComponent:@"f"]]);
    return finish();
}
