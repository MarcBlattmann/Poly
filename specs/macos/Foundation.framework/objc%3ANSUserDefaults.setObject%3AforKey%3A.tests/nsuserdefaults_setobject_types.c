/* poly-mac: foundation */
/* Test domain test.poly.nsspec.set. setObject:forKey: stores property list objects and the
 * typed readers convert (a number reads as integer, bool, double and, with
 * stringForKey:, as its string '42'; a string reads 0 as integer); an array and a
 * dictionary come back with their content, a Date and Data too; a mutable value
 * changed later does not change the stored one. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *suite = @"test.poly.nsspec.set";
    NSUserDefaults *u = [[NSUserDefaults alloc] initWithSuiteName:suite];
    [u removePersistentDomainForName:suite];
    [u setObject:@"str" forKey:@"s"];
    CK(ns_is([u objectForKey:@"s"], "str") && ns_is([u stringForKey:@"s"], "str"));
    [u setObject:@42 forKey:@"n"];
    CK([[u objectForKey:@"n"] isKindOfClass:[NSNumber class]] && [u integerForKey:@"n"] == 42 && [u boolForKey:@"n"] && [u doubleForKey:@"n"] == 42.0);
    CK(ns_is([u stringForKey:@"n"], "42"));
    CK([u integerForKey:@"s"] == 0);
    [u setBool:YES forKey:@"b"];
    CK([u objectForKey:@"b"] == (id)kCFBooleanTrue);
    [u setObject:@[@"a", @2] forKey:@"arr"];
    CK([[u arrayForKey:@"arr"] count] == 2 && ns_is([u arrayForKey:@"arr"][0], "a"));
    [u setObject:@{@"k": @"v"} forKey:@"d"];
    CK(ns_is([u dictionaryForKey:@"d"][@"k"], "v"));
    [u setObject:[NSData dataWithBytes:"abc" length:3] forKey:@"data"];
    CK([u dataForKey:@"data"].length == 3);
    [u setObject:[NSDate dateWithTimeIntervalSinceReferenceDate:1000] forKey:@"date"];
    CK([[u objectForKey:@"date"] timeIntervalSinceReferenceDate] == 1000.0);
    NSMutableString *m = [NSMutableString stringWithString:@"mut"];
    [u setObject:m forKey:@"m"];
    [m appendString:@"++"];
    CK(ns_is([u objectForKey:@"m"], "mut"));
    [u removePersistentDomainForName:suite];
    return finish();
}
