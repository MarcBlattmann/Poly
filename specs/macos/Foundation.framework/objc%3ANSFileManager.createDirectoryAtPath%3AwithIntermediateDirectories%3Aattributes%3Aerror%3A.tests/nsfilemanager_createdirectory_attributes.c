/* poly-mac: foundation */
/* Without attributes a new directory has the mode 0755 (the process's umask is
 * 022 here); with NSFilePosixPermissions the mode asked is the mode made
 * (0700); its type is NSFileTypeDirectory; the directory is empty and can be
 * removed with removeItemAtPath:error:. */
#include "poly_mfn.h"

int test_main(void) {
    NSFileManager *fm = [NSFileManager defaultManager];
    CK([fm createDirectoryAtPath:@"p1" withIntermediateDirectories:NO attributes:@{NSFilePosixPermissions: @0700} error:nil]);
    NSDictionary *a1 = [fm attributesOfItemAtPath:@"p1" error:nil];
    CK([a1[NSFilePosixPermissions] intValue] == 0700);
    CK(ns_is(a1[NSFileType], "NSFileTypeDirectory"));
    CK([fm createDirectoryAtPath:@"p2" withIntermediateDirectories:NO attributes:nil error:nil]);
    CK([[fm attributesOfItemAtPath:@"p2" error:nil][NSFilePosixPermissions] intValue] == 0755);
    CK([[fm contentsOfDirectoryAtPath:@"p2" error:nil] count] == 0);
    CK([fm removeItemAtPath:@"p2" error:nil] && ![fm fileExistsAtPath:@"p2"]);
    CK([fm createDirectoryAtPath:@"p3" withIntermediateDirectories:NO attributes:@{NSFilePosixPermissions: @0750} error:nil]);
    CK([[fm attributesOfItemAtPath:@"p3" error:nil][NSFilePosixPermissions] intValue] == 0750);
    return finish();
}
