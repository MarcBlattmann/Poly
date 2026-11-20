/* poly-mac: foundation */
/* Test domain test.poly.nsspec.rm. Setting a key again replaces it (also with
 * another type), setObject:nil and removeObjectForKey: remove it (removing a missing key
 * does nothing), a nil key and an empty key are accepted, synchronize answers
 * YES, and CoreFoundation's preferences see the same values (the user defaults
 * are CFPreferences). */
#include "poly_mfn.h"

int test_main(void) {
    NSString *suite = @"test.poly.nsspec.rm";
    NSUserDefaults *u = [[NSUserDefaults alloc] initWithSuiteName:suite];
    [u removePersistentDomainForName:suite];
    [u setObject:@"v1" forKey:@"k"];
    [u setObject:@"v2" forKey:@"k"];
    CK(ns_is([u objectForKey:@"k"], "v2"));
    [u setObject:@7 forKey:@"k"];
    CK([[u objectForKey:@"k"] isKindOfClass:[NSNumber class]]);
    id nothing = nil;
    [u setObject:nothing forKey:@"k"];
    CK([u objectForKey:@"k"] == nil);
    [u setObject:@"x" forKey:@"r"];
    [u removeObjectForKey:@"r"];
    [u removeObjectForKey:@"never"];
    CK([u objectForKey:@"r"] == nil);
    [u setObject:@"empty-key" forKey:@""];
    CK(ns_is([u objectForKey:@""], "empty-key"));
    [u setObject:@"cfside" forKey:@"c"];
    CK([u synchronize]);
    CFTypeRef cf = CFPreferencesCopyAppValue(CFSTR("c"), (__bridge CFStringRef)suite);
    CK(cf != NULL && cf_is(cf, "cfside"));
    if (cf) CFRelease(cf);
    CFPreferencesSetAppValue(CFSTR("fromcf"), CFSTR("viaCF"), (__bridge CFStringRef)suite);
    CK(ns_is([u objectForKey:@"fromcf"], "viaCF"));
    [u removePersistentDomainForName:suite];
    CK([u objectForKey:@"c"] == nil);
    return finish();
}
