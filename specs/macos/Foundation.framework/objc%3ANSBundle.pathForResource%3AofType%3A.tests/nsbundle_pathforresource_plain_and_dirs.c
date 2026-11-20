/* poly-mac: foundation */
/* Outside a bundle (the plain executable) every lookup is nil, including the
 * in-directory and URL forms; bundlePath of a plain directory opened with
 * bundleWithPath: finds files of that directory as resources (the directory itself is the
 * resource directory when it has no Contents). */
#include "poly_mfn.h"

int test_main(void) {
    NSBundle *main = [NSBundle mainBundle];
    CK([main pathForResource:@"hello" ofType:@"txt"] == nil);
    CK([main pathForResource:@"hello" ofType:@"txt" inDirectory:@"sub"] == nil);
    CK([main URLForResource:@"hello" withExtension:@"txt"] == nil);
    CK([main pathsForResourcesOfType:@"polyabsentext" inDirectory:nil].count == 0);
    [[NSFileManager defaultManager] createFileAtPath:@"data.txt" contents:[NSData dataWithBytes:"x" length:1] attributes:nil];
    NSBundle *dir = [NSBundle bundleWithPath:@"."];
    NSString *p = [dir pathForResource:@"data" ofType:@"txt"];
    CK(p != nil && [p hasSuffix:@"/data.txt"] && [p hasPrefix:@"/"]);
    CK([dir pathForResource:@"nodata" ofType:@"txt"] == nil);
    return finish();
}
