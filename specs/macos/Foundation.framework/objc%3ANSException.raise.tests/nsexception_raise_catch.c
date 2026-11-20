/* poly-mac: foundation */
/* -raise throws the exception: an @catch (NSException *) gets the same object, with its
 * name, reason and userInfo; @finally blocks run on the way; a nested @try rethrows
 * to the outer catch; raise:format: builds the reason with a format; @throw of a
 * non-exception object is caught as id; an exception that is only created is not
 * thrown and has no call stack until it is. */
#pragma clang diagnostic ignored "-Warc-performSelector-leaks"
#include "poly_mfn.h"

int test_main(void) {
    NSException *made = [NSException exceptionWithName:@"PolyEx" reason:@"because" userInfo:@{@"k": @"v"}];
    CK(made.callStackSymbols.count == 0);
    int caught = 0;
    @try {
        [made raise];
        CK(0);
    } @catch (NSException *e) {
        caught = 1;
        CK(e == made);
        CK(ns_is(e.name, "PolyEx") && ns_is(e.reason, "because") && ns_is(e.userInfo[@"k"], "v"));
        CK(e.callStackSymbols.count > 0);
    }
    CK(caught);
    int fin = 0;
    @try {
        @try {
            [[NSException exceptionWithName:@"Inner" reason:@"r" userInfo:nil] raise];
        } @finally {
            fin = 1;
        }
    } @catch (NSException *e) {
        CK(ns_is(e.name, "Inner"));
    }
    CK(fin);
    @try {
        [NSException raise:@"PolyFmt" format:@"v=%d s=%@", 7, @"x"];
    } @catch (NSException *e) {
        CK(ns_is(e.name, "PolyFmt") && ns_is(e.reason, "v=7 s=x") && e.userInfo == nil);
    }
    @try {
        @throw @"a string";
    } @catch (id o) {
        CK(ns_is(o, "a string"));
    }
    @try {
        [@"x" performSelector:NSSelectorFromString(@"noSuchMethod")];
    } @catch (NSException *e) {
        CK(ns_is(e.name, "NSInvalidArgumentException"));
    }
    return finish();
}
