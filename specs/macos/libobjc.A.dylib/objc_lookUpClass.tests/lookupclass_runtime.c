// poly-mac: libobjc
// objc_lookUpClass: runtime-made classes appear when registered, with several classes and after use.
#include "poly_mobjc.h"

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class a = objc_allocateClassPair(ns, "LuRtA", 0);
    check("A allocated", a != Nil, 0);
    check("A is not found before registration", objc_lookUpClass("LuRtA") == Nil, 0);
    objc_registerClassPair(a);
    check("A found", objc_lookUpClass("LuRtA") == a, 0);
    check("B is unknown before it exists", objc_lookUpClass("LuRtB") == Nil, 0);
    Class b = objc_allocateClassPair(a, "LuRtB", 0);
    check("B allocated", b != Nil, 0);
    check("B still not found", objc_lookUpClass("LuRtB") == Nil, 0);
    objc_registerClassPair(b);
    check("B found", objc_lookUpClass("LuRtB") == b, 0);
    check("B's superclass is A", class_getSuperclass(b) == a, 0);
    check("both agree with getClass", objc_getClass("LuRtA") == a && objc_getClass("LuRtB") == b, 0);
    /* a second allocation under a taken name fails; the first stays found */
    Class dup = objc_allocateClassPair(ns, "LuRtA", 0);
    check("duplicate name: no class", dup == Nil, 0);
    check("first still found", objc_lookUpClass("LuRtA") == a, 0);
    /* a name allocated but never registered stays unknown, and can be allocated again */
    Class c1 = objc_allocateClassPair(ns, "LuRtC", 0);
    check("C allocated", c1 != Nil, 0);
    check("C unknown", objc_lookUpClass("LuRtC") == Nil, 0);
    Class c2 = objc_allocateClassPair(ns, "LuRtC", 0);
    value("allocating the same unregistered name twice gives a class", c2 != Nil);
    return finish();
}
