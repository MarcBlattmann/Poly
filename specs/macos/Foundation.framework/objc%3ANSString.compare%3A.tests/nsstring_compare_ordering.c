/* poly-mac: foundation */
/* compare: orders by UTF-16 unit like CFStringCompare (NSOrderedAscending -1,
 * Same 0, Descending 1): 'a' < 'b', 'a' > 'B', 'Z' < 'a', a prefix is smaller,
 * the empty string is smaller; options select case-insensitive, numeric and
 * literal comparison; an unequal pair is never 0. */
#include "poly_mfn.h"

int test_main(void) {
    CKV([@"a" compare:@"b"] == NSOrderedAscending, [@"a" compare:@"b"]);
    CKV([@"b" compare:@"a"] == NSOrderedDescending, [@"b" compare:@"a"]);
    CKV([@"a" compare:@"a"] == NSOrderedSame, [@"a" compare:@"a"]);
    CKV([@"a" compare:@"B"] == NSOrderedDescending, [@"a" compare:@"B"]);
    CKV([@"Z" compare:@"a"] == NSOrderedAscending, [@"Z" compare:@"a"]);
    CKV([@"ab" compare:@"abc"] == NSOrderedAscending, 0);
    CKV([@"abc" compare:@"ab"] == NSOrderedDescending, 0);
    CKV([@"" compare:@"a"] == NSOrderedAscending, 0);
    CKV([@"" compare:@""] == NSOrderedSame, 0);
    CKV([@"a2" compare:@"a10"] == NSOrderedDescending, 0);
    CKV([@"a2" compare:@"a10" options:NSNumericSearch] == NSOrderedAscending, 0);
    CKV([@"a" compare:@"B" options:NSCaseInsensitiveSearch] == NSOrderedAscending, 0);
    CKV([@"A" compare:@"a" options:NSCaseInsensitiveSearch] == NSOrderedSame, 0);
    CKV([@"É" compare:@"é" options:NSCaseInsensitiveSearch] == NSOrderedSame, 0);
    CKV([@"a" caseInsensitiveCompare:@"A"] == NSOrderedSame, 0);
    CKV([@"a2" compare:@"a10" options:NSNumericSearch | NSCaseInsensitiveSearch] == NSOrderedAscending, 0);
    CK(NSOrderedAscending == -1 && NSOrderedSame == 0 && NSOrderedDescending == 1);
    CK(NSCaseInsensitiveSearch == 1 && NSLiteralSearch == 2 && NSBackwardsSearch == 4 && NSNumericSearch == 64);
    return finish();
}
