// poly-mac: libobjc
#include "poly_mobjc.h"
#include <pthread.h>
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
@interface Ex : NSObject
@end
@implementation Ex
@end
static id obj;
static volatile int result = -99;
static void *probe(void *arg) { (void)arg; result = objc_sync_exit(obj); return 0; }
static int exit_from_other_thread(void) { result = -99; pthread_t t; pthread_create(&t, 0, probe, 0); pthread_join(t, 0); return result; }
static void thrower(id e) { objc_exception_throw(e); }
int test_main(void) {
    obj = new_object();
    id e = objc_alloc_init(objc_getClass("Ex"));
    // the compiler's @synchronized: an exception leaves the lock
    @try { @synchronized (obj) { thrower(e); } } @catch (id x) { check("exception passed through @synchronized", x == e, 0); }
    value("exit by this thread after the exception (lock was released)", objc_sync_exit(obj));
    // inside @synchronized the lock is held by this thread
    @synchronized (obj) { value("exit from another thread inside @synchronized (not its owner)", exit_from_other_thread()); }
    // normal leave, break, return
    for (int i = 0; i < 3; i++) { @synchronized (obj) { if (i == 1) break; } }
    value("exit after leaving @synchronized by break", objc_sync_exit(obj));
    // nested @synchronized on the same object and on others
    @synchronized (obj) { @synchronized (obj) { @synchronized (e) { check("nested blocks run", 1, 0); } } }
    value("nothing held after nested blocks", objc_sync_exit(obj));
    // @synchronized(nil) does nothing
    id n = nil;
    @synchronized (n) { check("@synchronized (nil) runs the body", 1, 0); }
    // the object is evaluated once; it is the object at entry that is unlocked
    id first = obj;
    @synchronized (first) { first = e; }
    value("the object evaluated at entry is the one unlocked", objc_sync_exit(obj));
    value("and the other object was never locked", objc_sync_exit(e));
    rel(obj); rel(e);
    return finish();
}
