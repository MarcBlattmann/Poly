/* poly-mac: foundation */
/* Names with spaces and non-ASCII characters are found as written (the path is
 * UTF-8 on disk), composed or decomposed alike: the file exists under the string that
 * made it and under the decomposed spelling. On the oracle's APFS volume the
 * directory listing returns the name decomposed (e + U+0301, as ls does) whichever
 * way it was created. */
#include "poly_mfn.h"

int test_main(void) {
    NSFileManager *fm = [NSFileManager defaultManager];
    NSString *name = @"café €.txt";
    CK([fm createFileAtPath:name contents:[NSData dataWithBytes:"x" length:1] attributes:nil]);
    CK([fm fileExistsAtPath:name]);
    NSArray *list = [fm contentsOfDirectoryAtPath:@"." error:nil];
    CK(list.count == 1 && ns_is(list[0], "cafe\xcc\x81 \xe2\x82\xac.txt"));
    CK([fm fileExistsAtPath:@"cafe\u0301 \u20ac.txt"]);
    CK([fm fileExistsAtPath:[NSString stringWithUTF8String:"caf\xc3\xa9 \xe2\x82\xac.txt"]]);
    CK(![fm fileExistsAtPath:@"café"]);
    return finish();
}
