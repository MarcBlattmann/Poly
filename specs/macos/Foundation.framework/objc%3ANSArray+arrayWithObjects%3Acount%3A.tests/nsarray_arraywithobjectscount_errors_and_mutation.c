/* poly-mac: foundation */
/* A nil among the objects raises NSInvalidArgumentException, an index past the end
 * raises NSRangeException, and an immutable array does not answer addObject:
 * (unrecognized selector, NSInvalidArgumentException); NSMutableArray adds,
 * inserts and removes, and a mutable copy leaves the original alone. */
#include "poly_mfn.h"

static NSString *caught(void (^block)(void)) {
    @try {
        block();
    } @catch (NSException *e) {
        return e.name;
    }
    return nil;
}

int test_main(void) {
    NSString *r = caught(^{
        id bad[2] = {@"a", nil};
        (void)[NSArray arrayWithObjects:bad count:2];
    });
    CK(r != nil && ns_is(r, "NSInvalidArgumentException"));
    NSArray *arr = @[@"x", @"y"];
    r = caught(^{ (void)[arr objectAtIndex:5]; });
    CK(r != nil && ns_is(r, "NSRangeException"));
    r = caught(^{ [(NSMutableArray *)arr addObject:@"z"]; });
    CK(r != nil && ns_is(r, "NSInvalidArgumentException"));
    NSMutableArray *m = [NSMutableArray arrayWithArray:arr];
    [m addObject:@"c"];
    [m insertObject:@"z" atIndex:0];
    [m removeObjectAtIndex:1];
    CK(ns_is([m componentsJoinedByString:@","], "z,y,c"));
    CK(arr.count == 2);
    NSMutableArray *copy = [arr mutableCopy];
    [copy removeAllObjects];
    CK(copy.count == 0 && arr.count == 2);
    id nothing = nil;
    r = caught(^{ [m addObject:nothing]; });
    CK(r != nil && ns_is(r, "NSInvalidArgumentException"));
    return finish();
}
