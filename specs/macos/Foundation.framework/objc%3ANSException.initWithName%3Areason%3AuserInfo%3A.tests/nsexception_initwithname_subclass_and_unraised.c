/* poly-mac: foundation */
/* An NSException subclass keeps what the initialiser got and can be thrown and caught as
 * its own class; an exception caught by class (not by NSException) skips the
 * clauses before; an exception that is never thrown costs nothing and is freed
 * with its pool. */
#include "poly_mfn.h"

@interface PolyError : NSException
@end
@implementation PolyError
@end

int test_main(void) {
    PolyError *e = [[PolyError alloc] initWithName:@"PolyError" reason:@"custom" userInfo:@{@"code": @7}];
    CK(ns_is(e.name, "PolyError") && ns_is(e.reason, "custom") && [e.userInfo[@"code"] intValue] == 7);
    int which = 0;
    @try {
        @throw e;
    } @catch (NSArray *a) {
        which = 1;
    } @catch (PolyError *p) {
        which = 2;
        CK(p == e);
    } @catch (NSException *x) {
        which = 3;
    }
    CK(which == 2);
    @try {
        @throw [NSException exceptionWithName:@"Plain" reason:nil userInfo:nil];
    } @catch (PolyError *p) {
        which = 4;
    } @catch (NSException *x) {
        which = 5;
        CK(ns_is(x.name, "Plain"));
    }
    CK(which == 5);
    @autoreleasepool {
        (void)[NSException exceptionWithName:@"Never" reason:@"thrown" userInfo:nil];
    }
    CK([e isKindOfClass:[NSException class]]);
    return finish();
}
