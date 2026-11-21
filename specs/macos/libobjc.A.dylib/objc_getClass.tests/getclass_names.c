// poly-mac: libobjc
// objc_getClass: unusual names, long names, similar names.
#include "poly_mobjc.h"

static Class make(const char *name) {
    Class c = objc_allocateClassPair(objc_getClass("NSObject"), name, 0);
    if (c) objc_registerClassPair(c);
    return c;
}

int test_main(void) {
    const char *odd[] = {"Gn.Dot", "Gn-Dash", "Gn Space", "Gn$Dollar", "Gn_Under_", "Gn0123", "Gn\xc3\xa9", "Gn:colon", "G", "Gn<Proto>"};
    Class made[10];
    for (int i = 0; i < 10; i++) {
        made[i] = make(odd[i]);
        char nm[48] = "made ";
        int n = 5;
        for (int k = 0; odd[i][k] && n < 40; k++) nm[n++] = (odd[i][k] >= 32 && odd[i][k] < 127) ? odd[i][k] : '?';
        nm[n] = 0;
        check(nm, made[i] != Nil, i);
    }
    for (int i = 0; i < 10; i++) {
        char nm[48] = "found ";
        int n = 6;
        for (int k = 0; odd[i][k] && n < 40; k++) nm[n++] = (odd[i][k] >= 32 && odd[i][k] < 127) ? odd[i][k] : '?';
        nm[n] = 0;
        check(nm, objc_getClass(odd[i]) == made[i], i);
    }
    /* a long name */
    char big[1025];
    for (int i = 0; i < 1024; i++) big[i] = 'a' + (i % 26);
    big[1024] = 0;
    big[0] = 'G';
    Class lc = make(big);
    check("1024-character name registered", lc != Nil, 0);
    check("1024-character name found", objc_getClass(big) == lc, 0);
    value("its name length", str_len(class_getName(lc)));
    check("a name differing in the last character is unknown", (big[1023] = 'Z', objc_getClass(big) == Nil), 0);
    /* names are compared in full */
    make("GnPre");
    make("GnPrefix");
    check("GnPre", objc_getClass("GnPre") != objc_getClass("GnPrefix"), 0);
    check("GnPrefi is unknown", objc_getClass("GnPrefi") == Nil, 0);
    /* class and metaclass share the name; the metaclass is not found by name through getClass */
    Class m = object_getClass((id)objc_getClass("GnPre"));
    check("metaclass has the same name", str_eq(class_getName(m), "GnPre"), 0);
    check("getClass does not return the metaclass", objc_getClass("GnPre") != m, 0);
    return finish();
}
