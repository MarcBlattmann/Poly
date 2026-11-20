/* poly-mac: foundation */
/* createDirectoryAtPath:withIntermediateDirectories:attributes:error: without
 * intermediates fails when the parent is missing (NSCocoaErrorDomain, code 4)
 * or the directory exists (code 516), with them it makes the whole chain and
 * succeeds also when it exists; the error pointer may be NULL; over a file it
 * fails with 516. */
#include "poly_mfn.h"

int test_main(void) {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSError *err = nil;
    CK([fm createDirectoryAtPath:@"d1" withIntermediateDirectories:NO attributes:nil error:&err] && err == nil);
    err = nil;
    CK(![fm createDirectoryAtPath:@"d1" withIntermediateDirectories:NO attributes:nil error:&err]);
    CK(err != nil && ns_is(err.domain, "NSCocoaErrorDomain") && err.code == 516);
    err = nil;
    CK([fm createDirectoryAtPath:@"d1" withIntermediateDirectories:YES attributes:nil error:&err] && err == nil);
    err = nil;
    CK(![fm createDirectoryAtPath:@"a/b/c" withIntermediateDirectories:NO attributes:nil error:&err]);
    CK(err != nil && ns_is(err.domain, "NSCocoaErrorDomain") && err.code == 4);
    CK(![fm fileExistsAtPath:@"a"]);
    CK([fm createDirectoryAtPath:@"a/b/c" withIntermediateDirectories:YES attributes:nil error:&err]);
    BOOL isDir = NO;
    CK([fm fileExistsAtPath:@"a/b" isDirectory:&isDir] && isDir && [fm fileExistsAtPath:@"a/b/c" isDirectory:&isDir] && isDir);
    CK([fm createDirectoryAtPath:@"n1" withIntermediateDirectories:NO attributes:nil error:NULL]);
    [fm createFileAtPath:@"file" contents:nil attributes:nil];
    err = nil;
    CK(![fm createDirectoryAtPath:@"file" withIntermediateDirectories:YES attributes:nil error:&err]);
    CK(err != nil && err.code == 516);
    return finish();
}
