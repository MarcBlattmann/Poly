/* poly-mac: foundation */
/* Test domain test.poly.nsspec.init (and .other), cleaned at the end. A defaults
 * object made with initWithSuiteName: reads and writes that domain: another object
 * on the same suite sees the value, an object on another suite does not, nothing
 * is there at first, and the standard defaults (a different domain) are
 * not touched. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *suite = @"test.poly.nsspec.init", *other = @"test.poly.nsspec.other";
    NSUserDefaults *a = [[NSUserDefaults alloc] initWithSuiteName:suite];
    CK(a != nil);
    [a removePersistentDomainForName:suite];
    CK([a objectForKey:@"b"] == nil);
    [a setBool:YES forKey:@"b"];
    NSUserDefaults *b = [[NSUserDefaults alloc] initWithSuiteName:suite];
    CK([b boolForKey:@"b"]);
    NSUserDefaults *o = [[NSUserDefaults alloc] initWithSuiteName:other];
    CK([o objectForKey:@"b"] == nil);
    CK([[NSUserDefaults standardUserDefaults] objectForKey:@"b"] == nil);
    CK([NSUserDefaults standardUserDefaults] == [NSUserDefaults standardUserDefaults]);
    CK(ns_is(CFBridgingRelease(CFPreferencesCopyAppValue(CFSTR("b"), CFSTR("test.poly.nsspec.init"))) ? @"seen" : @"unseen", "seen"));
    [a removePersistentDomainForName:suite];
    [o removePersistentDomainForName:other];
    CK([a objectForKey:@"b"] == nil);
    return finish();
}
