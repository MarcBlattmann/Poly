/* poly-mac: corefoundation */
/* An array lists its elements' descriptions one per line, indented by four
 * spaces, in parentheses; a dictionary lists key = value; pairs in braces. */
#include "poly_mcf.h"

static void d(const char *name, CFTypeRef o) {
    CFStringRef s = CFCopyDescription(o);
    char buf[512], flat[512];
    cf_c(s, buf, sizeof buf);
    int j = 0;
    for (int i = 0; buf[i] && j < 500; i++) {
        if (buf[i] == '\n') { flat[j++] = '\\'; flat[j++] = 'n'; }
        else flat[j++] = buf[i];
    }
    flat[j] = 0;
    value_str(name, flat);
}

int test_main(void) {
    const void *two[2] = {CFSTR("x"), CFSTR("y")};
    d("array of two", CFArrayCreate(kCFAllocatorDefault, two, 2, &kCFTypeArrayCallBacks));
    const void *one[1] = {CFSTR("only")};
    d("array of one", CFArrayCreate(kCFAllocatorDefault, one, 1, &kCFTypeArrayCallBacks));
    const void *k[1] = {CFSTR("key")}, *v[1] = {CFSTR("value")};
    d("dictionary of one", CFDictionaryCreate(kCFAllocatorDefault, k, v, 1, &kCFTypeDictionaryKeyCallBacks,
                                              &kCFTypeDictionaryValueCallBacks));
    d("empty dictionary", CFDictionaryCreate(kCFAllocatorDefault, NULL, NULL, 0, &kCFTypeDictionaryKeyCallBacks,
                                             &kCFTypeDictionaryValueCallBacks));
    int n = 5;
    const void *num[1] = {CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &n)};
    d("array of a number", CFArrayCreate(kCFAllocatorDefault, num, 1, &kCFTypeArrayCallBacks));
    return finish();
}
