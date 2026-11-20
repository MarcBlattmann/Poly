/* An Objective-C program for Poly's own libobjc.A.dylib (ADR 0261), written
 * for Poly: a class with instance variables and properties, +load and
 * +initialize, a category, a subclass with a super send, ARC (strong and
 * weak references, dealloc), autorelease pools, run-time additions (a
 * method, a class pair, an associated object) and the @synchronized lock
 * calls. Built the ordinary way against the macOS SDK (generate.sh); on
 * Poly it runs with Poly's dyld, libSystem and libobjc. Prints one line
 * per check; returns the number of failures. */
#import <objc/runtime.h>
#import <objc/message.h>
#import <objc/NSObject.h>
#include <stdio.h>
#include <string.h>

extern int objc_sync_enter(id);
extern int objc_sync_exit(id);
extern void *objc_autoreleasePoolPush(void);
extern void objc_autoreleasePoolPop(void *);

/* -retain and -autorelease, which ARC code may not name. */
static void retain_autorelease(id o) {
    ((id (*)(id, SEL))objc_msgSend)(o, sel_registerName("retain"));
    ((id (*)(id, SEL))objc_msgSend)(o, sel_registerName("autorelease"));
}

static int failures;
static void check(const char *what, int ok) {
    printf("%s %s\n", ok ? "ok" : "not ok", what);
    if (!ok) failures++;
}

static int loads, inits, deallocs;

@interface Counter : NSObject {
@public
    int value;
}
@property (nonatomic, strong) id other;
- (instancetype)initWithValue:(int)v;
- (int)increment;
+ (instancetype)counterWithValue:(int)v;
@end

@implementation Counter
+ (void)load { loads++; }
+ (void)initialize {
    if (self == [Counter class]) inits++;
}
- (instancetype)initWithValue:(int)v {
    if ((self = [super init])) value = v;
    return self;
}
+ (instancetype)counterWithValue:(int)v { return [[self alloc] initWithValue:v]; }
- (int)increment { return ++value; }
- (void)dealloc { deallocs++; }
@end

@interface Counter (Doubling)
- (int)doubled;
@end
@implementation Counter (Doubling)
- (int)doubled { return value * 2; }
@end

@interface Sub : Counter
@end
@implementation Sub
- (int)increment { return [super increment] + 100; }
@end

static int answer(id self, SEL _cmd) { return 77; }

__attribute__((noinline)) static Counter *make_autoreleased(int v) {
    /* Returned at +0 the ARC way: the caller's handshake keeps it out of
     * the pool. */
    return [Counter counterWithValue:v];
}

int main(void) {
    @autoreleasepool {
        check("+load ran before main", loads == 1);
        Counter *c = [[Counter alloc] initWithValue:41];
        check("+initialize ran once, on the first message", inits == 1);
        check("a method", [c increment] == 42);
        check("a category method", [c doubled] == 84);
        Sub *s = [[Sub alloc] initWithValue:1];
        check("a super send", [s increment] == 102);
        check("isKindOfClass:", [s isKindOfClass:[Counter class]] && ![c isKindOfClass:[Sub class]]);
        check("respondsToSelector:", [c respondsToSelector:@selector(doubled)] &&
                                         ![c respondsToSelector:sel_registerName("nope")]);
        check("class_getName and the superclass", strcmp(class_getName([s class]), "Sub") == 0 &&
                                                      class_getSuperclass([Sub class]) == [Counter class]);
        check("NSObject's image is libobjc",
              strcmp(class_getImageName([NSObject class]), "/usr/lib/libobjc.A.dylib") == 0);

        __weak Counter *w = c;
        check("a weak reference reads the object", w == c);
        c.other = s;
        s = nil;
        check("a strong property keeps its object", deallocs == 0 && [c.other increment] == 103);
        c = nil;
        check("the last strong reference deallocs, the owner first", deallocs == 2);
        check("the weak reference is zeroed", w == nil);

        __weak Counter *w2;
        @autoreleasepool {
            Counter *t = make_autoreleased(5);
            w2 = t;
            check("a returned object lives", [w2 increment] == 6);
        }
        check("it dies at the end of its scope", w2 == nil && deallocs == 3);

        __weak id w3;
        void *pool = objc_autoreleasePoolPush();
        {
            id o = [Counter counterWithValue:9];
            retain_autorelease(o);
            w3 = o;
        }
        check("an autoreleased object waits for its pool", w3 != nil && deallocs == 3);
        objc_autoreleasePoolPop(pool);
        check("the pool's pop releases it", w3 == nil && deallocs == 4);

        check("class_addMethod", class_addMethod([Counter class], sel_registerName("answer"), (IMP)answer, "i@:"));
        Counter *d = [Counter counterWithValue:0];
        check("the added method answers", ((int (*)(id, SEL))objc_msgSend)(d, sel_registerName("answer")) == 77);

        Class made = objc_allocateClassPair([NSObject class], "Made", 0);
        class_addMethod(made, sel_registerName("answer"), (IMP)answer, "i@:");
        objc_registerClassPair(made);
        id m = [[made alloc] init];
        check("a class made at run time", objc_getClass("Made") == made &&
                                              ((int (*)(id, SEL))objc_msgSend)(m, sel_registerName("answer")) == 77);

        static char key;
        objc_setAssociatedObject(d, &key, m, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        check("an associated object", objc_getAssociatedObject(d, &key) == m);

        check("@synchronized's lock", objc_sync_enter(d) == 0 && objc_sync_enter(d) == 0 &&
                                          objc_sync_exit(d) == 0 && objc_sync_exit(d) == 0 &&
                                          objc_sync_exit(d) == -1);
    }
    check("everything freed", deallocs == 5);
    printf("objc: %s\n", failures ? "failed" : "done");
    return failures;
}
