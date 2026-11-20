// poly-mac: libobjc
// The classes and protocols the executable's Objective-C sections define are
// registered before main: they are found by name, appear in the class list,
// are attributed to the executable's image (and not to libobjc's, where
// NSObject lives), come with their metaclasses and their superclass links,
// and the compiled protocols are registered and adopted.
#include "poly_mobjc.h"
#include <dlfcn.h>

@protocol Walker
- (void)walk;
@end
@protocol Runner <Walker>
- (void)run;
@end

@interface Animal : NSObject <Walker>
@end
@interface Dog : Animal <Runner>
@end
@interface Cat : Animal
@end
@implementation Animal
- (void)walk {}
@end
@implementation Dog
- (void)run {}
@end
@implementation Cat
@end

static int has_name(Class *list, unsigned n, const char *name) {
    for (unsigned i = 0; i < n; i++) if (str_eq(class_getName(list[i]), name)) return 1;
    return 0;
}

int test_main(void) {
    Class animal = objc_getClass("Animal"), dog = objc_getClass("Dog"), cat = objc_getClass("Cat");
    check("Animal is found by name", animal != 0, 0);
    check("Dog is found by name", dog != 0, 0);
    check("Dog's superclass is Animal", class_getSuperclass(dog) == animal, 0);
    check("Animal's superclass is NSObject", class_getSuperclass(animal) == objc_getClass("NSObject"), 0);
    check("a class and its metaclass", object_getClass((id)dog) != (Class)dog && class_isMetaClass(object_getClass((id)dog)), 0);
    check("the metaclass chain follows the classes", class_getSuperclass(object_getClass((id)dog)) == object_getClass((id)animal), 0);
    check("[Dog class] is the registered class", [Dog class] == dog, 0);

    /* The class list. */
    int total = objc_getClassList(0, 0);
    value("a class list is not empty", total > 0);
    check("there is a class list", total > 0, total);
    Class *all = (Class *)malloc((unsigned long)total * sizeof(Class));
    int got = objc_getClassList(all, total);
    check("it holds as many as asked", got >= total, got);
    check("Animal, Dog, Cat are in it", has_name(all, (unsigned)total, "Animal") && has_name(all, (unsigned)total, "Dog") && has_name(all, (unsigned)total, "Cat"), 0);
    check("NSObject is in it", has_name(all, (unsigned)total, "NSObject"), 0);
    unsigned dup = 0;
    for (int i = 0; i < total; i++) for (int j = i + 1; j < total; j++) if (all[i] == all[j]) dup++;
    check("no class twice", dup == 0, dup);
    free(all);

    /* Image attribution. */
    const char *exe = class_getImageName(animal);
    check("a class of the executable has an image name", exe != 0, 0);
    check("Dog and Cat are of the same image", exe && class_getImageName(dog) && str_eq(exe, class_getImageName(dog)) && str_eq(exe, class_getImageName(cat)), 0);
    const char *objc = class_getImageName(objc_getClass("NSObject"));
    check("NSObject is of libobjc's image", objc != 0 && str_eq(objc, "/usr/lib/libobjc.A.dylib"), 0);
    check("not the executable's image", exe && objc && !str_eq(exe, objc), 0);
    unsigned count = 0;
    const char **names = exe ? objc_copyClassNamesForImage(exe, &count) : 0;
    check("the executable's classes can be listed", names != 0, count);
    value("classes of the executable", count);
    char order[64]; int on = 0;
    for (unsigned i = 0; i < count && on < 50; i++) {
        const char *nm = names[i];
        while (*nm) order[on++] = *nm++;
        order[on++] = ' ';
    }
    order[on] = 0;
    value_str("their names, in order", order);
    free(names);

    /* A class made at run time belongs to no image. */
    Class rt = objc_allocateClassPair(objc_getClass("NSObject"), "PolyRuntimeMade", 0);
    objc_registerClassPair(rt);
    check("a class made at run time has no image name", class_getImageName(rt) == 0, 0);

    /* Protocols. */
    Protocol *walker = objc_getProtocol("Walker"), *runner = objc_getProtocol("Runner");
    check("compiled protocols are registered", walker != 0 && runner != 0, 0);
    check("Animal conforms to Walker", class_conformsToProtocol(animal, walker), 0);
    check("Dog conforms to Runner", class_conformsToProtocol(dog, runner), 0);
    check("Dog conforms to Walker (through Animal and Runner)", class_conformsToProtocol(dog, walker), 0);
    check("Cat does not conform to Runner", !class_conformsToProtocol(cat, runner), 0);
    check("Runner extends Walker", protocol_conformsToProtocol(runner, walker), 0);
    value_str("protocol name", protocol_getName(runner));
    check("an unknown protocol is nil", objc_getProtocol("NoSuchProtocolAnywhere") == 0, 0);
    return finish();
}
