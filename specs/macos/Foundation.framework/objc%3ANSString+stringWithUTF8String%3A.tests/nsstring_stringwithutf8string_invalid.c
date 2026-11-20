/* poly-mac: foundation */
/* Bytes that are not well-formed UTF-8 give nil: stray lead or continuation
 * bytes, a truncated sequence, an overlong form, an encoded surrogate, a
 * code point beyond U+10FFFF. Valid text next to them is not affected. */
#include "poly_mfn.h"

int test_main(void) {
    CK([NSString stringWithUTF8String:"\xff\xfe"] == nil);
    CK([NSString stringWithUTF8String:"\x80"] == nil);
    CK([NSString stringWithUTF8String:"ab\xc3"] == nil);
    CK([NSString stringWithUTF8String:"\xc0\x80"] == nil);
    CK([NSString stringWithUTF8String:"\xed\xa0\x80"] == nil);
    CK([NSString stringWithUTF8String:"\xf4\x90\x80\x80"] == nil);
    CK([NSString stringWithUTF8String:"\xf0\x9f\x98"] == nil);
    CK([NSString stringWithUTF8String:"fine"] != nil);
    return finish();
}
