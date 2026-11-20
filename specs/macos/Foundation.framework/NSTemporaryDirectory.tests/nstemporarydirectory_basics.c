/* poly-mac: foundation */
/* NSTemporaryDirectory is an absolute path with a trailing slash, to an existing,
 * writable directory that belongs to the user (a per-user directory, below
 * /var/folders on macOS), the same on every call; a file can be created in it and
 * removed. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *tmp = NSTemporaryDirectory();
    BOOL isDir = NO;
    CK(tmp != nil && [tmp hasPrefix:@"/"] && [tmp hasSuffix:@"/"]);
    CK([[NSFileManager defaultManager] fileExistsAtPath:tmp isDirectory:&isDir] && isDir);
    CK([NSTemporaryDirectory() isEqualToString:tmp]);
    CK([tmp hasPrefix:@"/var/folders/"] || [tmp hasPrefix:@"/private/var/folders/"]);
    NSString *file = [tmp stringByAppendingPathComponent:@"poly-nsspec-tmp-probe.txt"];
    NSFileManager *fm = [NSFileManager defaultManager];
    CK([fm createFileAtPath:file contents:[NSData dataWithBytes:"x" length:1] attributes:nil]);
    CK([fm isWritableFileAtPath:tmp]);
    CK([fm removeItemAtPath:file error:nil] && ![fm fileExistsAtPath:file]);
    return finish();
}
