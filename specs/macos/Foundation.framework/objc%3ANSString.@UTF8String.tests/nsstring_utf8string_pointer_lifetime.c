/* poly-mac: foundation */
/* The pointer belongs to the string (or the current autorelease pool) and
 * stays valid at least until the pool is drained: it can be read after the
 * string was changed, for the strings of a mutable string made earlier. */
#include "poly_mfn.h"

int test_main(void) {
    NSMutableString *m = [NSMutableString stringWithString:@"before"];
    const char *p = m.UTF8String;
    [m setString:@"after the change, and a different length"];
    CK(mem_eq(p, "before", 7));
    CK(mem_eq(m.UTF8String, "after the change, and a different length", 41));
    const char *q;
    @autoreleasepool {
        q = [NSString stringWithFormat:@"%@-%d", @"pooled", 1].UTF8String;
        CK(mem_eq(q, "pooled-1", 9));
    }
    const char *outer = [NSString stringWithFormat:@"%@", @"outer"].UTF8String;
    CK(mem_eq(outer, "outer", 6));
    return finish();
}
