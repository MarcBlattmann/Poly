// poly-mac: libobjc
// method_getTypeEncoding: the encodings of NSObject's methods (the root class libobjc itself provides).
#include "poly_mobjc.h"

#define ENC(sel) value_str(sel, or_nil(method_getTypeEncoding(class_getInstanceMethod(ns, sel_registerName(sel)))))

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    ENC("init"); ENC("dealloc"); ENC("hash"); ENC("isEqual:"); ENC("self"); ENC("class"); ENC("superclass"); ENC("isKindOfClass:");
    ENC("isMemberOfClass:"); ENC("respondsToSelector:"); ENC("conformsToProtocol:"); ENC("retain"); ENC("release"); ENC("autorelease");
    ENC("retainCount"); ENC("zone"); ENC("copy"); ENC("mutableCopy"); ENC("description"); ENC("performSelector:"); ENC("performSelector:withObject:");
    ENC("isProxy"); ENC("forwardingTargetForSelector:"); ENC("doesNotRecognizeSelector:");
    Class m = object_getClass((id)ns);
    value_str("+alloc", or_nil(method_getTypeEncoding(class_getInstanceMethod(m, sel_registerName("alloc")))));
    value_str("+new", or_nil(method_getTypeEncoding(class_getInstanceMethod(m, sel_registerName("new")))));
    value_str("+allocWithZone:", or_nil(method_getTypeEncoding(class_getInstanceMethod(m, sel_registerName("allocWithZone:")))));
    value_str("+initialize", or_nil(method_getTypeEncoding(class_getInstanceMethod(m, sel_registerName("initialize")))));
    value_str("+instancesRespondToSelector:", or_nil(method_getTypeEncoding(class_getInstanceMethod(m, sel_registerName("instancesRespondToSelector:")))));
    value_str("+isSubclassOfClass:", or_nil(method_getTypeEncoding(class_getInstanceMethod(m, sel_registerName("isSubclassOfClass:")))));
    check("NSObject has an encoding for every method asked", method_getTypeEncoding(class_getInstanceMethod(ns, sel_registerName("init"))) != 0, 0);
    return finish();
}
