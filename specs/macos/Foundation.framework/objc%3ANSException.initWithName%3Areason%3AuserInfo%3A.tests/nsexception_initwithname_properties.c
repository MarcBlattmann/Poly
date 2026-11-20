/* poly-mac: foundation */
/* initWithName:reason:userInfo: keeps name, reason and userInfo as given (reason and
 * userInfo may be nil, the name is returned as is), the description is the
 * reason (or the name when there is none), two exceptions are different objects
 * even with equal content, it is an NSObject that can be thrown with @throw. */
#include "poly_mfn.h"

int test_main(void) {
    NSException *e = [[NSException alloc] initWithName:@"PolyName" reason:@"some reason" userInfo:@{@"a": @1}];
    CK(ns_is(e.name, "PolyName") && ns_is(e.reason, "some reason") && [e.userInfo[@"a"] intValue] == 1);
    NSException *bare = [[NSException alloc] initWithName:@"Bare" reason:nil userInfo:nil];
    CK(ns_is(bare.name, "Bare") && bare.reason == nil && bare.userInfo == nil);
    CK(ns_is(e.description, "some reason"));
    CK(ns_is(bare.description, "Bare"));
    NSException *twin = [[NSException alloc] initWithName:@"PolyName" reason:@"some reason" userInfo:@{@"a": @1}];
    CK(e != twin);
    CK(e.callStackReturnAddresses.count == 0 && e.callStackSymbols.count == 0);
    @try {
        @throw e;
    } @catch (NSException *c) {
        CK(c == e);
        CK(c.callStackSymbols.count > 0);
    }
    CK([e isKindOfClass:[NSException class]] && [e isKindOfClass:[NSObject class]]);
    NSException *named = [NSException exceptionWithName:@"Fac" reason:@"r" userInfo:nil];
    CK(ns_is(named.name, "Fac"));
    return finish();
}
