// poly-mac: libobjc
// objc_allocateClassPair: makes an unregistered class and metaclass; fails on taken names.
#include "poly_mobjc.h"

@interface ApBase : NSObject
@end
@implementation ApBase
@end

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class base = objc_getClass("ApBase");
    Class c = objc_allocateClassPair(base, "ApNew", 0);
    check("a class is returned", c != Nil, 0);
    value_str("name", class_getName(c));
    check("superclass is the one given", class_getSuperclass(c) == base, 0);
    Class meta = object_getClass((id)c);
    check("it has a metaclass", meta != Nil, 0);
    check("the metaclass is another object", meta != c, 0);
    check("metaclass is flagged as such", class_isMetaClass(meta) && !class_isMetaClass(c), 0);
    check("metaclass has the same name", str_eq(class_getName(meta), "ApNew"), 0);
    check("metaclass superclass is the superclass's metaclass", class_getSuperclass(meta) == object_getClass((id)base), 0);
    check("not findable before registration", objc_getClass("ApNew") == Nil, 0);

    check("name of a registered compiled class is taken", objc_allocateClassPair(base, "ApBase", 0) == Nil, 0);
    check("name NSObject is taken", objc_allocateClassPair(ns, "NSObject", 0) == Nil, 0);
    objc_registerClassPair(c);
    check("name of a runtime class is taken once registered", objc_allocateClassPair(base, "ApNew", 0) == Nil, 0);
    check("findable after registration", objc_getClass("ApNew") == c, 0);

    /* extra bytes are accepted */
    Class e = objc_allocateClassPair(ns, "ApExtra", 64);
    check("extraBytes 64 accepted", e != Nil, 0);
    objc_registerClassPair(e);
    id o = send0((id)e, "new");
    check("an instance of it works", o != nil && object_getClass(o) == e, 0);
    send0(o, "release");

    /* a superclass that is not registered is refused */
    Class unreg = objc_allocateClassPair(ns, "ApUnreg", 0);
    check("unregistered class allocated", unreg != Nil, 0);
    check("an unregistered superclass gives nil", objc_allocateClassPair(unreg, "ApUnregChild", 0) == Nil, 0);
    objc_registerClassPair(unreg);
    check("after registering it, the subclass can be allocated", objc_allocateClassPair(unreg, "ApUnregChild", 0) != Nil, 0);
    return finish();
}
