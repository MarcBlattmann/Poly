// poly-mac: libobjc
// objc_msgSend with a tagged-pointer receiver. An object that is a tagged
// pointer carries its class and value in the pointer itself: the runtime
// finds the class from the tag bits (after undoing the per-process
// obfuscation), dispatches as usual, and reference counting does nothing to
// it. Foundation's NSNumber and NSString make such objects, so this test
// loads Foundation at run time (dlopen) and says what it saw if it cannot.
#include "poly_mobjc.h"
#include <dlfcn.h>

extern unsigned long objc_debug_taggedpointer_obfuscator;
extern unsigned long objc_debug_taggedpointer_mask;

static int is_tagged(id o) {
    return ((unsigned long)o ^ objc_debug_taggedpointer_obfuscator) & objc_debug_taggedpointer_mask ? 1 : 0;
}

int test_main(void) {
    void *h = dlopen("/System/Library/Frameworks/Foundation.framework/Foundation", RTLD_NOW);
    check("Foundation loads", h != 0, 0);
    if (!h) return finish();
    Class number = objc_getClass("NSNumber");
    Class string = objc_getClass("NSString");
    check("NSNumber exists", number != 0, 0);

    id n5 = ((id (*)(Class, SEL, int))objc_msgSend)(number, sel_registerName("numberWithInt:"), 5);
    check("a small NSNumber is a tagged pointer", is_tagged(n5), 0);
    value_str("its class", cname(n5));
    check("intValue", ((int (*)(id, SEL))objc_msgSend)(n5, sel_registerName("intValue")) == 5, 0);
    check("doubleValue", ((double (*)(id, SEL))objc_msgSend)(n5, sel_registerName("doubleValue")) == 5.0, 0);
    check("boolValue", ((BOOL (*)(id, SEL))objc_msgSend)(n5, sel_registerName("boolValue")) == YES, 0);
    check("another number", ((int (*)(id, SEL))objc_msgSend)(((id (*)(Class, SEL, int))objc_msgSend)(number, sel_registerName("numberWithInt:"), -7), sel_registerName("intValue")) == -7, 0);
    check("isKindOfClass: NSNumber", ((BOOL (*)(id, SEL, Class))objc_msgSend)(n5, sel_registerName("isKindOfClass:"), number) == YES, 0);
    check("-class is the same as object_getClass", ((Class (*)(id, SEL))objc_msgSend)(n5, sel_registerName("class")) == object_getClass(n5), 0);
    check("-self returns the pointer", ((id (*)(id, SEL))objc_msgSend)(n5, sel_registerName("self")) == n5, 0);
    check("object_isClass is false", !object_isClass(n5), 0);

    /* Reference counting is a no-op: the pointer is returned unchanged and
     * any number of releases is harmless. */
    check("objc_retain returns the same pointer", objc_retain(n5) == n5, 0);
    for (int i = 0; i < 5; i++) objc_release(n5);
    check("still usable after extra releases", ((int (*)(id, SEL))objc_msgSend)(n5, sel_registerName("intValue")) == 5, 0);
    check("objc_autorelease returns the same pointer", objc_autorelease(n5) == n5, 0);

    /* Two equal numbers made separately are the same tagged pointer. */
    id n5b = ((id (*)(Class, SEL, int))objc_msgSend)(number, sel_registerName("numberWithInt:"), 5);
    check("equal small numbers are the same pointer", n5 == n5b, 0);

    /* A value too big for the tag is an ordinary heap object. */
    id big = ((id (*)(Class, SEL, long long))objc_msgSend)(number, sel_registerName("numberWithLongLong:"), 0x7fffffffffffffffLL);
    check("a big NSNumber is not tagged", !is_tagged(big), 0);
    check("and still answers", ((long long (*)(id, SEL))objc_msgSend)(big, sel_registerName("longLongValue")) == 0x7fffffffffffffffLL, 0);

    /* A short ASCII string is tagged too. */
    id s = ((id (*)(Class, SEL, const char *))objc_msgSend)(string, sel_registerName("stringWithUTF8String:"), "ab");
    check("a short string is a tagged pointer", is_tagged(s), 0);
    value_str("its class", cname(s));
    check("length", ((unsigned long (*)(id, SEL))objc_msgSend)(s, sel_registerName("length")) == 2, 0);
    value_str("UTF8String", ((const char *(*)(id, SEL))objc_msgSend)(s, sel_registerName("UTF8String")));
    return finish();
}
