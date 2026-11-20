// poly-mac: libobjc
// class_getName of run-time classes: the name given to objc_allocateClassPair is copied; long and unusual names.
#include "poly_mobjc.h"

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    char buf[32] = "GnrCopied";
    Class c = objc_allocateClassPair(ns, buf, 0);
    buf[0] = 'Z'; buf[3] = 0;
    check("the returned name is the original", str_eq(class_getName(c), "GnrCopied"), 0);
    check("the returned pointer is not the caller's buffer", class_getName(c) != buf, 0);
    objc_registerClassPair(c);
    check("and after registration", str_eq(class_getName(c), "GnrCopied"), 0);
    char big[301];
    for (int i = 0; i < 300; i++) big[i] = 'A' + (i % 26);
    big[300] = 0;
    Class l = objc_allocateClassPair(ns, big, 0);
    objc_registerClassPair(l);
    check("300 characters", str_eq(class_getName(l), big), 0);
    value("length", str_len(class_getName(l)));
    Class u = objc_allocateClassPair(ns, "Gnr Spaces & Symbols!", 0);
    objc_registerClassPair(u);
    value_str("symbols", class_getName(u));
    Class e = objc_allocateClassPair(ns, "", 0);
    value("an empty name gives a class", e != Nil);
    if (e) value_str("its name", class_getName(e));
    /* subclasses keep their own names */
    Class s = objc_allocateClassPair(c, "GnrChild", 0);
    objc_registerClassPair(s);
    check("subclass name", str_eq(class_getName(s), "GnrChild") && str_eq(class_getName(class_getSuperclass(s)), "GnrCopied"), 0);
    return finish();
}
