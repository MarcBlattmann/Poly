/* poly-mac: foundation */
/* A number holds its value and gives it as every scalar type: intValue,
 * longLongValue, doubleValue, boolValue, the string value; converting
 * truncates toward zero and wraps for narrower integer types; numbers of different
 * creation types with the same value are equal and hash equally, and compare:
 * orders by value. */
#include "poly_mfn.h"

int test_main(void) {
    NSNumber *n = [NSNumber numberWithInt:42];
    CK(n.intValue == 42 && n.longLongValue == 42 && n.doubleValue == 42.0 && n.boolValue);
    CK(ns_is(n.stringValue, "42") && ns_is(n.description, "42"));
    CK([NSNumber numberWithInt:-7].intValue == -7);
    CK(![NSNumber numberWithInt:0].boolValue);
    CK([@2.75 intValue] == 2 && [@(-2.75) intValue] == -2);
    CK([@300 charValue] == 44);
    CK([@(1LL << 40) longLongValue] == (1LL << 40));
    CK([@1 isEqual:@1LL] && [@1 isEqual:@1.0] && ![@1 isEqual:@1.5]);
    CK([@1 isEqualToNumber:@1.0]);
    CK([@5 hash] == [@5LL hash] && [@5 hash] == [@5.0 hash]);
    CK([@1 compare:@2] == NSOrderedAscending && [@2 compare:@1] == NSOrderedDescending && [@1 compare:@1.0] == NSOrderedSame);
    CK(ns_is([@1.5 description], "1.5") && ns_is([@YES description], "1"));
    CK([@(NAN) isEqual:@(NAN)]);
    return finish();
}
