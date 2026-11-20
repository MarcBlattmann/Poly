// poly-mac: libobjc
#include "poly_mobjc.h"
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static volatile long zero;
int test_main(void) {
    id a = new_object(), b = new_object();
    value("OBJC_SYNC_SUCCESS", OBJC_SYNC_SUCCESS);
    value("OBJC_SYNC_NOT_OWNING_THREAD_ERROR", OBJC_SYNC_NOT_OWNING_THREAD_ERROR);
    value("retainCount before", (i64)retain_count(a));
    value("enter a", objc_sync_enter(a));
    value("retainCount while locked (the lock does not retain)", (i64)retain_count(a));
    value("enter a again (recursive)", objc_sync_enter(a));
    value("enter b (another object is independent)", objc_sync_enter(b));
    value("enter nil", objc_sync_enter((id)zero));
    value("exit nil", objc_sync_exit((id)zero));
    value("exit b", objc_sync_exit(b));
    value("exit a", objc_sync_exit(a));
    value("exit a again", objc_sync_exit(a));
    value("exit a a third time (no lock is held)", objc_sync_exit(a));
    value("enter a class object", objc_sync_enter((id)objc_getClass("NSObject")));
    value("exit a class object", objc_sync_exit((id)objc_getClass("NSObject")));
    value("enter the metaclass", objc_sync_enter(object_getClass((id)objc_getClass("NSObject"))));
    value("exit the metaclass", objc_sync_exit(object_getClass((id)objc_getClass("NSObject"))));
    value("retainCount after", (i64)retain_count(a));
    rel(a); rel(b);
    return finish();
}
