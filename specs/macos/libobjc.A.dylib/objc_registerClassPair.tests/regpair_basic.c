// poly-mac: libobjc
// objc_registerClassPair: the class becomes usable; superclass methods are inherited; class methods work.
#include "poly_mobjc.h"

static long forty_two(id self, SEL _cmd) { (void)self; (void)_cmd; return 42; }
static long class_forty_three(id self, SEL _cmd) { (void)self; (void)_cmd; return 43; }
static long tag_of(id self, SEL _cmd) { (void)_cmd; return (long)class_getInstanceSize(object_getClass(self)); }

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    Class c = objc_allocateClassPair(ns, "RgBasic", 0);
    class_addMethod(c, sel_registerName("answer"), (IMP)forty_two, "q@:");
    Class meta = object_getClass((id)c);
    class_addMethod(meta, sel_registerName("classAnswer"), (IMP)class_forty_three, "q@:");
    class_addMethod(c, sel_registerName("size"), (IMP)tag_of, "q@:");
    check("not found yet", objc_getClass("RgBasic") == Nil, 0);
    objc_registerClassPair(c);
    check("found", objc_getClass("RgBasic") == c, 0);
    check("metaclass unchanged by registration", object_getClass((id)c) == meta, 0);
    check("still a class and metaclass pair", !class_isMetaClass(c) && class_isMetaClass(meta), 0);
    check("superclass is NSObject", class_getSuperclass(c) == ns, 0);
    check("meta superclass is NSObject's metaclass", class_getSuperclass(meta) == object_getClass((id)ns), 0);
    value("instance size equals NSObject's", class_getInstanceSize(c) == class_getInstanceSize(ns));
    check("class method works", send0l((id)c, "classAnswer") == 43, 0);
    id o = send0((id)c, "new");
    check("instance made through inherited +new", o != nil, 0);
    check("object_getClass(o) is the class", object_getClass(o) == c, 0);
    check("instance method works", send0l(o, "answer") == 42, 0);
    check("inherited -class", (Class)send0(o, "class") == c, 0);
    check("inherited -isKindOfClass: NSObject", ((BOOL(*)(id, SEL, Class))objc_msgSend)(o, sel_registerName("isKindOfClass:"), ns), 0);
    check("inherited -isMemberOfClass:", ((BOOL(*)(id, SEL, Class))objc_msgSend)(o, sel_registerName("isMemberOfClass:"), c), 0);
    check("inherited -respondsToSelector:", ((BOOL(*)(id, SEL, SEL))objc_msgSend)(o, sel_registerName("respondsToSelector:"), sel_registerName("answer")), 0);
    check("instance does not answer the class method", ((BOOL(*)(id, SEL, SEL))objc_msgSend)(o, sel_registerName("respondsToSelector:"), sel_registerName("classAnswer")) == 0, 0);
    value("size method", send0l(o, "size") == (long)class_getInstanceSize(ns));
    send0(o, "release");
    return finish();
}
