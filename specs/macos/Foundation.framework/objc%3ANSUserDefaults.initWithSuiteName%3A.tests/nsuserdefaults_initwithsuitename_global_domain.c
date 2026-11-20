/* poly-mac: foundation */
/* Test domain test.poly.nsspec.global. A suite also reads the user's global
 * values (AppleLanguages is visible through it); the object answers the typed getters of a
 * key that is not there with the zero value (0, NO, 0.0, nil) and a nil
 * suite name gives the standard defaults object's domain or a usable object. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *suite = @"test.poly.nsspec.global";
    NSUserDefaults *u = [[NSUserDefaults alloc] initWithSuiteName:suite];
    [u removePersistentDomainForName:suite];
    CK([u objectForKey:@"AppleLanguages"] != nil);
    CK([u integerForKey:@"nokey"] == 0 && ![u boolForKey:@"nokey"] && [u doubleForKey:@"nokey"] == 0.0);
    CK([u stringForKey:@"nokey"] == nil && [u arrayForKey:@"nokey"] == nil && [u dictionaryForKey:@"nokey"] == nil && [u dataForKey:@"nokey"] == nil);
    CK([u dictionaryRepresentation][@"nokey"] == nil);
    NSUserDefaults *nilsuite = [[NSUserDefaults alloc] initWithSuiteName:nil];
    CK(nilsuite != nil);
    [u removePersistentDomainForName:suite];
    return finish();
}
