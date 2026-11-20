// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static char k1, k2;
static char karr[8];
static volatile long zero;
int test_main(void) {
    id o1 = new_object(), o2 = new_object(), v = new_object(), w = new_object();
    id nilo = (id)zero;
    const void *nilk = (const void *)zero;
    // nil owner
    check("get on nil owner", objc_getAssociatedObject(nilo, &k1) == 0, 0);
    objc_setAssociatedObject(nilo, &k1, nil, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    check("set of nil on nil owner returns", 1, 0);
    // independent owners
    objc_setAssociatedObject(o1, &k1, v, OBJC_ASSOCIATION_ASSIGN);
    check("o1 has it", objc_getAssociatedObject(o1, &k1) == v, 0);
    check("o2 does not", objc_getAssociatedObject(o2, &k1) == 0, 0);
    // independent keys; the key is compared by address, never dereferenced
    objc_setAssociatedObject(o1, &k2, w, OBJC_ASSOCIATION_ASSIGN);
    check("k1 value kept", objc_getAssociatedObject(o1, &k1) == v, 0);
    check("k2 value", objc_getAssociatedObject(o1, &k2) == w, 0);
    objc_setAssociatedObject(o1, (void *)1, v, OBJC_ASSOCIATION_ASSIGN);
    check("key 1 (not an address)", objc_getAssociatedObject(o1, (void *)1) == v, 0);
    objc_setAssociatedObject(o1, nilk, w, OBJC_ASSOCIATION_ASSIGN);
    check("key NULL works as a key", objc_getAssociatedObject(o1, nilk) == w, 0);
    objc_setAssociatedObject(o1, "abc", v, OBJC_ASSOCIATION_ASSIGN);
    check("an equal string at another address is another key", objc_getAssociatedObject(o1, (const char *)karr) == 0, 0);
    // the selector as key is the usual idiom
    objc_setAssociatedObject(o1, @selector(hash), w, OBJC_ASSOCIATION_ASSIGN);
    check("selector as key", objc_getAssociatedObject(o1, @selector(hash)) == w, 0);
    // setting nil with a different policy removes
    objc_setAssociatedObject(o1, &k1, nil, OBJC_ASSOCIATION_RETAIN);
    check("removed", objc_getAssociatedObject(o1, &k1) == 0, 0);
    // remove everything
    objc_removeAssociatedObjects(o1);
    check("k2 removed by objc_removeAssociatedObjects", objc_getAssociatedObject(o1, &k2) == 0, 0);
    check("key 1 removed by objc_removeAssociatedObjects", objc_getAssociatedObject(o1, (void *)1) == 0, 0);
    // ASSIGN of an object that is later freed: the pointer stays (dangling) and is not zeroed
    rel(o1); rel(o2); rel(v); rel(w);
    return finish();
}
