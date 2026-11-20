/* poly-mac: corefoundation */
/* True gives true (1) and false gives false (0), after being stored in and
 * read from collections as well. */
#include "poly_mcf.h"

int test_main(void) {
    CK(CFBooleanGetValue(kCFBooleanTrue) == 1);
    CK(CFBooleanGetValue(kCFBooleanFalse) == 0);
    CFArrayRef a = CFArrayCreate(kCFAllocatorDefault, (const void *[]){kCFBooleanFalse, kCFBooleanTrue}, 2, &kCFTypeArrayCallBacks);
    CK(!CFBooleanGetValue(CFArrayGetValueAtIndex(a, 0)));
    CK(CFBooleanGetValue(CFArrayGetValueAtIndex(a, 1)));
    CK(sizeof(Boolean) == 1);
    for (int i = 0; i < 3; i++) {
        CFRelease(kCFBooleanFalse);
        CK(CFBooleanGetValue(kCFBooleanFalse) == 0);
    }
    return finish();
}
