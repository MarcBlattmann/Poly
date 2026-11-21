// poly-mac: libobjc
// sel_getName: names of selectors made at run time, of methods listed from a class, and the "no selector" text.
#include "poly_mobjc.h"
#include <stdlib.h>

static SEL volatile null_sel;
static long nop(id self, SEL _cmd) { (void)self; (void)_cmd; return 0; }

int test_main(void) {
    Class c = objc_allocateClassPair(objc_getClass("NSObject"), "SnmCls", 0);
    class_addMethod(c, sel_registerName("snmFirst"), (IMP)nop, "q@:");
    class_addMethod(c, sel_registerName("snmSecond:with:"), (IMP)nop, "q@:qq");
    class_addMethod(c, sel_registerName("snmThird:"), (IMP)nop, "q@:q");
    objc_registerClassPair(c);
    unsigned n = 0;
    Method *l = class_copyMethodList(c, &n);
    value("own methods listed", n);
    int seen1 = 0, seen2 = 0, seen3 = 0;
    for (unsigned i = 0; i < n; i++) {
        const char *s = sel_getName(method_getName(l[i]));
        if (str_eq(s, "snmFirst")) seen1 = 1;
        if (str_eq(s, "snmSecond:with:")) seen2 = 1;
        if (str_eq(s, "snmThird:")) seen3 = 1;
    }
    free(l);
    check("all three names read back from listed methods", seen1 && seen2 && seen3, 0);
    SEL s = sel_registerName("snmRuntime:x:");
    check("runtime name", str_eq(sel_getName(s), "snmRuntime:x:"), 0);
    check("getName then register gives the same selector", sel_registerName(sel_getName(s)) == s, 0);
    check("the null selector's text", str_eq(sel_getName(null_sel), "<null selector>"), 0);
    check("an empty-name selector gives an empty string", sel_getName(sel_registerName("")) != 0 && sel_getName(sel_registerName(""))[0] == 0, 0);
    check("a message selector sent through the runtime keeps its name", str_eq(sel_getName(@selector(snmCompiled:)), "snmCompiled:"), 0);
    char buf[24] = "snmMutable";
    SEL m = sel_registerName(buf);
    buf[0] = 0;
    check("the registry keeps its own copy of the name", str_eq(sel_getName(m), "snmMutable"), 0);
    return finish();
}
