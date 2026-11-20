/* poly-mac: foundation */
/* The other path helpers on the same strings: pathExtension (after the last dot
 * of the last component; none for '.hidden', 'a.' or a dot in a directory),
 * stringByDeletingLastPathComponent, stringByDeletingPathExtension,
 * stringByAppendingPathExtension:, pathComponents and pathWithComponents:. */
#include "poly_mfn.h"

int test_main(void) {
    CK(ns_is([@"a.txt" pathExtension], "txt"));
    CK(ns_is([@"a.tar.gz" pathExtension], "gz"));
    CK(ns_is([@".hidden" pathExtension], ""));
    CK(ns_is([@"a" pathExtension], ""));
    CK(ns_is([@"a." pathExtension], ""));
    CK(ns_is([@"/d.x/f" pathExtension], ""));
    CK(ns_is([@"/a/b/c" stringByDeletingLastPathComponent], "/a/b"));
    CK(ns_is([@"/a" stringByDeletingLastPathComponent], "/"));
    CK(ns_is([@"a" stringByDeletingLastPathComponent], ""));
    CK(ns_is([@"/" stringByDeletingLastPathComponent], "/"));
    CK(ns_is([@"a.txt" stringByDeletingPathExtension], "a"));
    CK(ns_is([@"a" stringByAppendingPathExtension:@"txt"], "a.txt"));
    NSArray *pc = [@"/a/b/c" pathComponents];
    CK(pc.count == 4 && ns_is(pc[0], "/") && ns_is(pc[1], "a") && ns_is(pc[3], "c"));
    NSArray *rel = [@"a/b//c/" pathComponents];
    CK(rel.count == 4 && ns_is(rel[0], "a") && ns_is(rel[2], "c") && ns_is(rel[3], "/"));
    CK(ns_is([NSString pathWithComponents:@[@"/", @"a", @"b"]], "/a/b"));
    CK(ns_is([@"/a/./b/../c//d" stringByStandardizingPath], "/a/c/d"));
    CK([@"/a" isAbsolutePath] && ![@"a/b" isAbsolutePath]);
    return finish();
}
