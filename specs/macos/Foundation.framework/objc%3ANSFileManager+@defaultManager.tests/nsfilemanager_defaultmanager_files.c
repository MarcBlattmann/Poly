/* poly-mac: foundation */
/* The same manager creates and lists files: a file made with createFileAtPath:
 * contents:attributes: exists, has its size in attributesOfItemAtPath: (type
 * NSFileTypeRegular), is listed, and removeItemAtPath:error: deletes it; removing
 * it twice fails with NSCocoaErrorDomain code 4 (no such file); copyItemAtPath: copies
 * the bytes. */
#include "poly_mfn.h"

int test_main(void) {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSData *content = [NSData dataWithBytes:"hello" length:5];
    CK([fm createFileAtPath:@"a.txt" contents:content attributes:nil]);
    CK([fm fileExistsAtPath:@"a.txt"]);
    NSDictionary *at = [fm attributesOfItemAtPath:@"a.txt" error:nil];
    CK([at[NSFileSize] unsignedLongLongValue] == 5);
    CK(ns_is(at[NSFileType], "NSFileTypeRegular"));
    CK([[fm contentsOfDirectoryAtPath:@"." error:nil] isEqualToArray:@[@"a.txt"]]);
    NSError *err = nil;
    CK([fm copyItemAtPath:@"a.txt" toPath:@"b.txt" error:&err] && err == nil);
    CK([[NSData dataWithContentsOfFile:@"b.txt"] isEqualToData:content]);
    CK([fm removeItemAtPath:@"a.txt" error:nil] && ![fm fileExistsAtPath:@"a.txt"]);
    err = nil;
    CK(![fm removeItemAtPath:@"a.txt" error:&err]);
    CK(err != nil && ns_is(err.domain, "NSCocoaErrorDomain") && err.code == 4);
    CK(![fm copyItemAtPath:@"b.txt" toPath:@"b.txt" error:&err]);
    return finish();
}
