/* poly-mac: foundation */
/* stringWithUTF8String: decodes UTF-8 into a string of UTF-16 units (é one unit,
 * a character above U+FFFF two); the empty C string gives an empty string, not
 * nil; a leading byte order mark is dropped, one inside stays. The units are
 * read through CoreFoundation. */
#include "poly_mfn.h"

int test_main(void) {
    NSString *s = [NSString stringWithUTF8String:"h\xc3\xa9llo"];
    CK(s != nil && CFStringGetLength((__bridge CFStringRef)s) == 5);
    CK(CFStringGetCharacterAtIndex((__bridge CFStringRef)s, 1) == 0x00e9);
    CK(ns_is(s, "h\xc3\xa9llo"));
    NSString *e = [NSString stringWithUTF8String:"a\xf0\x9f\x98\x80z"];
    CK(CFStringGetLength((__bridge CFStringRef)e) == 4);
    CK(CFStringGetCharacterAtIndex((__bridge CFStringRef)e, 1) == 0xd83d && CFStringGetCharacterAtIndex((__bridge CFStringRef)e, 2) == 0xde00);
    NSString *empty = [NSString stringWithUTF8String:""];
    CK(empty != nil && CFStringGetLength((__bridge CFStringRef)empty) == 0);
    CK(CFStringGetLength((__bridge CFStringRef)[NSString stringWithUTF8String:"\xef\xbb\xbf" "a"]) == 1);
    CK(CFStringGetLength((__bridge CFStringRef)[NSString stringWithUTF8String:"a\xef\xbb\xbf" "b"]) == 3);
    CK([[NSString stringWithUTF8String:"same"] isEqualToString:@"same"]);
    return finish();
}
