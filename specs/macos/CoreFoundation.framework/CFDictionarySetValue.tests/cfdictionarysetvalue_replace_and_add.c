/* poly-mac: corefoundation */
/* SetValue adds a pair or replaces the value of an equal key (the count stays);
 * AddValue only adds, ReplaceValue only replaces, RemoveValue ignores a key
 * that is not there. */
#include "poly_mcf.h"

int test_main(void) {
    CFMutableDictionaryRef d = CFDictionaryCreateMutable(kCFAllocatorDefault, 0, &kCFTypeDictionaryKeyCallBacks,
                                                         &kCFTypeDictionaryValueCallBacks);
    CFDictionarySetValue(d, CFSTR("k"), CFSTR("v1"));
    CK(CFDictionaryGetCount(d) == 1 && cf_is(CFDictionaryGetValue(d, CFSTR("k")), "v1"));
    CFDictionarySetValue(d, S("k"), CFSTR("v2"));
    CK(CFDictionaryGetCount(d) == 1 && cf_is(CFDictionaryGetValue(d, CFSTR("k")), "v2"));
    CFDictionaryAddValue(d, CFSTR("k"), CFSTR("v3"));
    CK(cf_is(CFDictionaryGetValue(d, CFSTR("k")), "v2"));
    CFDictionaryAddValue(d, CFSTR("j"), CFSTR("w"));
    CK(CFDictionaryGetCount(d) == 2);
    CFDictionaryReplaceValue(d, CFSTR("nokey"), CFSTR("x"));
    CK(CFDictionaryGetCount(d) == 2 && !CFDictionaryContainsKey(d, CFSTR("nokey")));
    CFDictionaryReplaceValue(d, CFSTR("j"), CFSTR("w2"));
    CK(cf_is(CFDictionaryGetValue(d, CFSTR("j")), "w2"));
    CFDictionaryRemoveValue(d, CFSTR("j"));
    CFDictionaryRemoveValue(d, CFSTR("none"));
    CK(CFDictionaryGetCount(d) == 1 && !CFDictionaryContainsKey(d, CFSTR("j")));
    return finish();
}
