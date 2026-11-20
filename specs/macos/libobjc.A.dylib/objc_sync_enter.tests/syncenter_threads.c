// poly-mac: libobjc
#include "poly_mobjc.h"
#include <pthread.h>
static void rel(id o) { ((void(*)(id, SEL))objc_msgSend)(o, sel_registerName("release")); }
static id lock_obj;
static long counter, in_section, overlaps;
static void *adder(void *arg) {
    (void)arg;
    for (int i = 0; i < 20000; i++) {
        objc_sync_enter(lock_obj);
        if (++in_section != 1) overlaps++;
        long c = counter; for (volatile int k = 0; k < 3; k++) { } counter = c + 1;
        in_section--;
        objc_sync_exit(lock_obj);
    }
    return 0;
}
static volatile int stage;
static volatile int got;
static void *waiter(void *arg) {
    (void)arg;
    stage = 1;
    int r = objc_sync_enter(lock_obj);
    got = 1;
    objc_sync_exit(lock_obj);
    return (void *)(long)r;
}
int test_main(void) {
    lock_obj = new_object();
    pthread_t t[4];
    for (int i = 0; i < 4; i++) pthread_create(&t[i], 0, adder, 0);
    for (int i = 0; i < 4; i++) pthread_join(t[i], 0);
    value("counter after 4 x 20000 locked increments", counter);
    value("times two threads were inside at once", overlaps);
    // a second thread waits until the owner exits
    objc_sync_enter(lock_obj);
    pthread_t w; pthread_create(&w, 0, waiter, 0);
    while (!stage) { }
    for (volatile long k = 0; k < 50000000; k++) { }
    value("waiter got the lock while main holds it", got);
    objc_sync_enter(lock_obj);
    objc_sync_exit(lock_obj);
    for (volatile long k = 0; k < 20000000; k++) { }
    value("waiter after main's inner exit (still held once)", got);
    objc_sync_exit(lock_obj);
    void *r; pthread_join(w, &r);
    value("waiter got the lock after main's last exit", got);
    value("waiter's enter result", (long)r);
    // a different object is not blocked
    id other = new_object();
    objc_sync_enter(lock_obj);
    check("other object: enter works while lock_obj is held by this thread", objc_sync_enter(other) == 0, 0);
    objc_sync_exit(other); objc_sync_exit(lock_obj);
    rel(other); rel(lock_obj);
    return finish();
}
