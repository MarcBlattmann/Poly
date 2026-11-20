/* poly-mac: foundation */
/* Appending in a chain builds a path one slash at a time, and what is built can be
 * taken apart again with the other path helpers; appending to the result of
 * stringByDeletingLastPathComponent returns the sibling. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *p = @"/";
    for (NSString *c in @[@"usr", @"local", @"bin", @"tool"]) p = [p stringByAppendingPathComponent:c];
    CK(ns_is(p, "/usr/local/bin/tool"));
    CK(ns_is(p.lastPathComponent, "tool"));
    CK(ns_is([p.stringByDeletingLastPathComponent stringByAppendingPathComponent:@"other"], "/usr/local/bin/other"));
    CK([p pathComponents].count == 5);
    CK(ns_is([@"rel" stringByAppendingPathComponent:@"a"], "rel/a"));
    CK(ns_is([[@"a" stringByAppendingPathComponent:@"b"] stringByAppendingPathComponent:@"c"], "a/b/c"));
    return finish();
}
