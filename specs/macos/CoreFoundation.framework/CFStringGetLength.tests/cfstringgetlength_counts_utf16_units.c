/* poly-mac: corefoundation */
/* The length is the number of UTF-16 code units: a character above U+FFFF
 * counts two, a combining mark counts one, and so do NUL and the byte order
 * mark. */
#include "poly_mcf.h"

int test_main(void) {
    CK(CFStringGetLength(CFSTR("")) == 0);
    CK(CFStringGetLength(CFSTR("hello")) == 5);
    CK(CFStringGetLength(S("h\xc3\xa9llo")) == 5);
    CK(CFStringGetLength(S("\xf0\x9f\x98\x80")) == 2);
    CK(CFStringGetLength(S("a\xf0\x9f\x98\x80\xf0\x9f\x98\x80z")) == 6);
    UniChar comb[2] = {0x65, 0x301}, nul[3] = {0x41, 0, 0x42}, bom[2] = {0xfeff, 0x61};
    CK(CFStringGetLength(CFStringCreateWithCharacters(kCFAllocatorDefault, comb, 2)) == 2);
    CK(CFStringGetLength(CFStringCreateWithCharacters(kCFAllocatorDefault, nul, 3)) == 3);
    CK(CFStringGetLength(CFStringCreateWithCharacters(kCFAllocatorDefault, bom, 2)) == 2);
    CK(CFStringGetLength(CFSTR("Aé€")) == 3);
    return finish();
}
