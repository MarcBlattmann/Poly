// poly-mac: libobjc
// Categories of the executable are attached to their classes before main
// (and before any +load): their methods are in the class's method list,
// found by class_getInstanceMethod and answered by instances. A category
// method replaces the class's own method of the same selector; of two
// categories that define the same selector the one later in the image wins;
// categories can add class methods and methods to classes defined elsewhere
// (NSObject).
#include "poly_mobjc.h"

static int load_saw;     /* what a +load saw of the category methods */

@interface K : NSObject
- (int)own;
- (int)shadowed;
- (int)twice;
+ (int)cls;
@end
@interface K (One)
- (int)fromOne;
- (int)shadowed;
- (int)twice;
@end
@interface K (Two)
- (int)twice;
+ (int)cls;
@end
@interface NSObject (PolyCat)
- (int)onEverything;
@end

@implementation K
+ (void)load { load_saw = [self instancesRespondToSelector:@selector(fromOne)] ? 1 : 0; }
- (int)own { return 1; }
- (int)shadowed { return 2; }
- (int)twice { return 3; }
+ (int)cls { return 100; }
@end
@implementation K (One)
- (int)fromOne { return 10; }
- (int)shadowed { return 20; }
- (int)twice { return 30; }
@end
@implementation K (Two)
- (int)twice { return 31; }
+ (int)cls { return 101; }
@end
@implementation NSObject (PolyCat)
- (int)onEverything { return 5; }
@end

int test_main(void) {
    Class k = objc_getClass("K");
    id o = [K new];
    check("+load of the class saw the category method", load_saw == 1, load_saw);
    check("the class's own method", ((int (*)(id, SEL))objc_msgSend)(o, sel_registerName("own")) == 1, 0);
    check("a category adds a method", ((int (*)(id, SEL))objc_msgSend)(o, sel_registerName("fromOne")) == 10, 0);
    check("class_getInstanceMethod finds it", class_getInstanceMethod(k, sel_registerName("fromOne")) != 0, 0);
    check("a category method replaces the class's own", ((int (*)(id, SEL))objc_msgSend)(o, sel_registerName("shadowed")) == 20, 0);
    long twice = ((int (*)(id, SEL))objc_msgSend)(o, sel_registerName("twice"));
    value("two categories define -twice: the answer", twice);
    check("the later category in the image wins", twice == 31, twice);
    long cls = ((int (*)(id, SEL))objc_msgSend)((id)k, sel_registerName("cls"));
    value("a category's +cls over the class's", cls);
    check("a category class method replaces the class's", cls == 101, cls);
    check("a category on NSObject reaches every object", ((int (*)(id, SEL))objc_msgSend)(o, sel_registerName("onEverything")) == 5, 0);
    check("and every class object, through NSObject's instance methods", ((int (*)(id, SEL))objc_msgSend)((id)k, sel_registerName("onEverything")) == 5, 0);

    /* The class's own method, replaced by a category, is still in its list. */
    unsigned n = 0;
    Method *list = class_copyMethodList(k, &n);
    int own = 0, shadowed = 0;
    for (unsigned i = 0; i < n; i++) {
        if (method_getName(list[i]) == sel_registerName("shadowed")) shadowed++;
        if (method_getName(list[i]) == sel_registerName("own")) own++;
    }
    char order[200]; int on = 0;
    for (unsigned i = 0; i < n && on < 150; i++) {
        const char *nm = sel_getName(method_getName(list[i]));
        while (*nm) order[on++] = *nm++;
        order[on++] = ' ';
    }
    order[on] = 0;
    value_str("the class's method list, in order", order);
    value("methods in the class's list (own + categories)", n);
    value("entries named -shadowed in the list", shadowed);
    check("-own is listed once", own == 1, own);
    free(list);
    return finish();
}
