// poly-mac: libobjc
// object_getClass for run-time classes and instances made with class_createInstance and object_setClass.
#include "poly_mobjc.h"

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class a = objc_allocateClassPair(ns, "OgrA", 0);
    Class am = object_getClass((id)a);
    check("before registration the class has a metaclass", am != Nil && class_isMetaClass(am), 0);
    objc_registerClassPair(a);
    check("registration does not change it", object_getClass((id)a) == am, 0);
    check("metaclass's class is the root metaclass", object_getClass((id)am) == object_getClass((id)ns), 0);
    id o = class_createInstance(a, 0);
    check("class_createInstance: object_getClass", object_getClass(o) == a, 0);
    check("-class agrees", (Class)send0(o, "class") == a, 0);
    Class b = objc_allocateClassPair(ns, "OgrB", 0);
    objc_registerClassPair(b);
    Class prev = object_setClass(o, b);
    check("object_setClass returns the previous class", prev == a, 0);
    check("object_getClass now answers the new class", object_getClass(o) == b, 0);
    check("-class too", (Class)send0(o, "class") == b, 0);
    check("class_getName follows", str_eq(cname(o), "OgrB"), 0);
    /* the class of a class can be seen through a message too */
    check("-class on the metaclass-bearing class", (Class)send0((id)a, "class") == a, 0);
    object_dispose(o);
    /* instances of a deep chain */
    Class prevc = ns;
    char nm[16];
    Class last = Nil;
    for (int i = 0; i < 5; i++) {
        nm[0] = 'O'; nm[1] = 'g'; nm[2] = 'r'; nm[3] = 'D'; nm[4] = '0' + i; nm[5] = 0;
        last = objc_allocateClassPair(prevc, nm, 0);
        objc_registerClassPair(last);
        prevc = last;
    }
    id d = send0((id)last, "new");
    check("object five levels deep", object_getClass(d) == last, 0);
    Class m = object_getClass((id)last);
    int hops = 0;
    while (m != object_getClass((id)ns) && hops < 20) { m = class_getSuperclass(m); hops++; }
    value("metaclass hops to the root metaclass", hops);
    send0(d, "release");
    return finish();
}
