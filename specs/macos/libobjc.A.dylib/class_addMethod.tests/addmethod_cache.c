// poly-mac: libobjc
// class_addMethod after messages were sent: the new method is used by the next message (no stale cache).
#include "poly_mobjc.h"

static long over(id self, SEL _cmd) { (void)self; (void)_cmd; return 1234; }
static long on_sub(id self, SEL _cmd) { (void)self; (void)_cmd; return 5678; }

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class a = objc_allocateClassPair(ns, "AmCacheA", 0);
    objc_registerClassPair(a);
    Class b = objc_allocateClassPair(a, "AmCacheB", 0);
    objc_registerClassPair(b);
    SEL retainCount = sel_registerName("retainCount");
    id oa = send0((id)a, "new");
    id ob = send0((id)b, "new");
    for (int i = 0; i < 5; i++) { (void)send0l(oa, "retainCount"); (void)send0l(ob, "retainCount"); }
    check("before: inherited retainCount is 1 (a)", send0l(oa, "retainCount") == 1 || retain_count(oa) == 1, 0);
    check("before: inherited retainCount is 1 (b)", retain_count(ob) == 1, 0);
    check("add override to A", class_addMethod(a, retainCount, (IMP)over, "Q@:"), 0);
    check("A's next message uses it", send0l(oa, "retainCount") == 1234, 0);
    check("a subclass instance sees it too", send0l(ob, "retainCount") == 1234, 0);
    check("add override to B", class_addMethod(b, retainCount, (IMP)on_sub, "Q@:"), 0);
    check("B's own now wins", send0l(ob, "retainCount") == 5678, 0);
    check("A is unchanged by B's", send0l(oa, "retainCount") == 1234, 0);
    check("NSObject itself is unchanged", class_getInstanceMethod(ns, retainCount) != 0 && method_getImplementation(class_getInstanceMethod(ns, retainCount)) != (IMP)over, 0);
    id plain = send0((id)ns, "new");
    check("plain NSObject instance unchanged", send0l(plain, "retainCount") == 1, 0);

    /* class methods: added to the metaclass */
    SEL cm = sel_registerName("cmeth");
    check("class method added through the metaclass", class_addMethod(object_getClass((id)a), cm, (IMP)over, "Q@:"), 0);
    check("a class message uses it", send0l((id)a, "cmeth") == 1234, 0);
    check("subclass inherits the class method", send0l((id)b, "cmeth") == 1234, 0);
    check("instances do not respond", class_respondsToSelector(a, cm) == 0, 0);
    check("the metaclass does", class_respondsToSelector(object_getClass((id)a), cm), 0);
    object_dispose(oa); object_dispose(ob); object_dispose(plain);
    return finish();
}
