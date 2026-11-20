// poly-mac: libobjc
// objc_getClass: finds classes by name; nil for unknown names.
#include "poly_mobjc.h"

@interface GcAlpha : NSObject
@end
@implementation GcAlpha
@end
@interface GcBeta : GcAlpha
@end
@implementation GcBeta
@end

int test_main(void) {
    Class ns = objc_getClass("NSObject");
    check("NSObject is found", ns != Nil, 0);
    value_str("NSObject name", class_getName(ns));
    check("same answer twice", objc_getClass("NSObject") == ns, 0);
    check("[NSObject class] equals it", (Class)send0((id)ns, "class") == ns, 0);

    Class a = objc_getClass("GcAlpha");
    Class b = objc_getClass("GcBeta");
    check("compiled class GcAlpha found", a != Nil, 0);
    check("compiled class GcBeta found", b != Nil, 0);
    check("GcAlpha and GcBeta differ", a != b, 0);
    check("GcBeta superclass is GcAlpha", class_getSuperclass(b) == a, 0);
    check("GcAlpha superclass is NSObject", class_getSuperclass(a) == ns, 0);
    check("@class reference equals lookup", (Class)[GcAlpha class] == a, 0);

    check("unknown name gives nil", objc_getClass("GcNoSuchClass") == Nil, 0);
    check("empty name gives nil", objc_getClass("") == Nil, 0);
    check("lookup is case sensitive", objc_getClass("nsobject") == Nil, 0);
    check("lookup is case sensitive (2)", objc_getClass("gcalpha") == Nil, 0);
    check("prefix of a name is not enough", objc_getClass("GcAlph") == Nil, 0);
    check("name with trailing space is unknown", objc_getClass("GcAlpha ") == Nil, 0);

    return finish();
}
