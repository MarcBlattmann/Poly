// poly-mac: libobjc
#include "poly_mobjc.h"
#include <pthread.h>
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static id obj;
static volatile int ready, got;
static void *waiter(void *arg) { (void)arg; ready = 1; objc_sync_enter(obj); got = 1; objc_sync_exit(obj); return 0; }
static void spin(void) { for (volatile long k = 0; k < 30000000; k++) { } }
int test_main(void) {
    obj = new_object();
    for (int i = 0; i < 5; i++) objc_sync_enter(obj);
    pthread_t t; pthread_create(&t, 0, waiter, 0);
    while (!ready) { }
    int held = 0;
    for (int i = 0; i < 4; i++) { objc_sync_exit(obj); spin(); if (got) held++; }
    value("exits after which the waiter got in (of the first four)", held);
    value("waiter in before the last exit", got);
    objc_sync_exit(obj);
    pthread_join(t, 0);
    value("waiter in after the last exit", got);
    // 200 nested enters and exits
    int bad = 0;
    for (int i = 0; i < 200; i++) if (objc_sync_enter(obj)) bad++;
    for (int i = 0; i < 200; i++) if (objc_sync_exit(obj)) bad++;
    value("errors in 200 nested enters and exits", bad);
    value("one exit too many", objc_sync_exit(obj));
    rel(obj);
    return finish();
}
