// poly-mac: libobjc
// sel_registerName: one SEL per distinct name; equality is by value; the name reads back.
#include "poly_mobjc.h"

int test_main(void) {
    SEL a = sel_registerName("selregAlpha");
    SEL b = sel_registerName("selregAlpha");
    check("same name, same selector", a == b, 0);
    char copy[16] = "selregAlpha";
    check("same name in another buffer, same selector", sel_registerName(copy) == a, 0);
    check("a different name differs", sel_registerName("selregBeta") != a, 0);
    check("case matters", sel_registerName("SELREGALPHA") != a, 0);
    check("a prefix differs", sel_registerName("selregAlph") != a, 0);
    check("a longer name differs", sel_registerName("selregAlphaX") != a, 0);
    check("a selector with a colon differs from without", sel_registerName("selregAlpha:") != a, 0);
    check("@selector equals the registered one", @selector(selregViaSelector) == sel_registerName("selregViaSelector"), 0);
    check("a built-in selector: init", @selector(init) == sel_registerName("init"), 0);
    check("a built-in selector: dealloc", @selector(dealloc) == sel_registerName("dealloc"), 0);
    check("sel_getUid is the same function in effect", sel_getUid("selregAlpha") == a, 0);
    check("sel_isEqual", sel_isEqual(a, b) && !sel_isEqual(a, sel_registerName("selregBeta")), 0);
    check("never NULL", sel_registerName("selregGamma") != 0, 0);
    check("the empty string is a selector", sel_registerName("") != 0, 0);
    check("... always the same one", sel_registerName("") == sel_registerName(""), 0);
    check("... different from others", sel_registerName("") != a, 0);
    value_str("name of a", sel_getName(a));
    value_str("empty name round trip", sel_getName(sel_registerName("")));
    /* the buffer is copied */
    char buf[16] = "selregTemp";
    SEL t = sel_registerName(buf);
    buf[0] = 'X';
    check("the name was copied", str_eq(sel_getName(t), "selregTemp"), 0);
    /* many selectors */
    SEL many[200];
    char nm[16] = "selregN000";
    int ok = 1;
    for (int i = 0; i < 200; i++) {
        nm[7] = '0' + i / 100; nm[8] = '0' + (i / 10) % 10; nm[9] = '0' + i % 10;
        many[i] = sel_registerName(nm);
        for (int j = 0; j < i; j++) if (many[j] == many[i]) ok = 0;
    }
    check("200 distinct names give 200 distinct selectors", ok, 0);
    nm[7] = '0'; nm[8] = '4'; nm[9] = '2';
    check("and look up again", sel_registerName(nm) == many[42], 0);
    return finish();
}
