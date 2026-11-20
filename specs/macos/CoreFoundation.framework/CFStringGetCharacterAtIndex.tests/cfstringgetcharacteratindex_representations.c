/* poly-mac: corefoundation */
/* The same characters read the same however the string is stored: a constant,
 * a copy, a mutable one, a substring, and one made from bytes of another
 * encoding. */
#include "poly_mcf.h"

int test_main(void) {
    CFStringRef c = CFSTR("xyzé");
    CFMutableStringRef m = CFStringCreateMutableCopy(kCFAllocatorDefault, 0, c);
    CFStringRef sub = CFStringCreateWithSubstring(kCFAllocatorDefault, c, CFRangeMake(2, 2));
    CFStringRef l1 = CFStringCreateWithCString(kCFAllocatorDefault, "xyz\xe9", kCFStringEncodingISOLatin1);
    CKV(CFStringGetCharacterAtIndex(c, 3) == 0xe9, CFStringGetCharacterAtIndex(c, 3));
    CKV(CFStringGetCharacterAtIndex(m, 3) == 0xe9, CFStringGetCharacterAtIndex(m, 3));
    CKV(CFStringGetCharacterAtIndex(sub, 0) == 'z', CFStringGetCharacterAtIndex(sub, 0));
    CKV(CFStringGetCharacterAtIndex(sub, 1) == 0xe9, CFStringGetCharacterAtIndex(sub, 1));
    CKV(CFStringGetCharacterAtIndex(l1, 3) == 0xe9, CFStringGetCharacterAtIndex(l1, 3));
    CFStringAppend(m, CFSTR("!"));
    CKV(CFStringGetCharacterAtIndex(m, 4) == '!', CFStringGetCharacterAtIndex(m, 4));
    CFStringDelete(m, CFRangeMake(0, 1));
    CKV(CFStringGetCharacterAtIndex(m, 0) == 'y', CFStringGetCharacterAtIndex(m, 0));
    return finish();
}
