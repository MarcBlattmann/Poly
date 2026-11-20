/* poly-mac: foundation */
/* UTF8String gives a NUL-terminated UTF-8 C string: ASCII as is, é two bytes,
 * a character above U+FFFF one four-byte sequence, the empty string a pointer
 * to a single NUL; the same text from CFSTR constants, run-time strings and
 * mutable strings. The bytes are compared with mem_eq, not through Foundation. */
#include "poly_mfn.h"

int test_main(void) {
    const char *a = @"hello".UTF8String;
    CK(a != NULL && mem_eq(a, "hello", 6));
    const char *e = @"héllo".UTF8String;
    CK(e != NULL && mem_eq(e, "h\xc3\xa9llo", 7));
    const char *emoji = ns_make("x\xf0\x9f\x98\x80y").UTF8String;
    CK(emoji != NULL && mem_eq(emoji, "x\xf0\x9f\x98\x80y", 7));
    const char *euro = @"€".UTF8String;
    CK(mem_eq(euro, "\xe2\x82\xac", 4));
    const char *empty = @"".UTF8String;
    CK(empty != NULL && empty[0] == 0);
    CK(ns_make("").UTF8String[0] == 0);
    NSMutableString *m = [NSMutableString stringWithString:@"mut"];
    [m appendString:@"able"];
    CK(mem_eq(m.UTF8String, "mutable", 8));
    CK(mem_eq([[NSString stringWithFormat:@"%d", 12345] UTF8String], "12345", 6));
    CK(str_eq(ns_make("a long string, longer than any tagged short string could hold").UTF8String,
              "a long string, longer than any tagged short string could hold"));
    return finish();
}
