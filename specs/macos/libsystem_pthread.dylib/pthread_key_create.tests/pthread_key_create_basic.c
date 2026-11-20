/* pthread_key_create: new keys start with NULL in every thread; a
 * destructor runs when a thread with a non-NULL value ends; deleting a
 * key frees it for reuse and never runs destructors. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <stdint.h>

static int dcalls;
static void *dlast;
static void dtor(void *v) {
    dcalls++;
    dlast = v;
}
static pthread_key_t key;
static void *set_and_exit(void *arg) {
    pthread_setspecific(key, arg);
    return NULL;
}
static void *read_in_thread(void *arg) {
    (void)arg;
    return pthread_getspecific(key);
}

int test_main(void) {
    int r = pthread_key_create(&key, dtor);
    check("create returns 0", r == 0, r);
    check("a new key reads NULL", pthread_getspecific(key) == NULL, 0);
    pthread_setspecific(key, (void *)5);
    check("main's value reads back", (intptr_t)pthread_getspecific(key) == 5, 0);
    pthread_t t;
    void *rv = (void *)1;
    pthread_create(&t, NULL, read_in_thread, NULL);
    pthread_join(t, &rv);
    check("another thread sees NULL, not main's 5", rv == NULL, (long long)(intptr_t)rv);

    pthread_create(&t, NULL, set_and_exit, (void *)7);
    pthread_join(t, NULL);
    check("the destructor ran once for the ended thread", dcalls == 1, dcalls);
    check("with the thread's value", (intptr_t)dlast == 7, (long long)(intptr_t)dlast);
    pthread_create(&t, NULL, set_and_exit, NULL);
    pthread_join(t, NULL);
    check("a NULL value gets no destructor call", dcalls == 1, dcalls);

    check("delete returns 0", pthread_key_delete(key) == 0, 0);
    check("delete did not call the destructor for main's value", dcalls == 1, dcalls);

    pthread_key_t k2;
    pthread_key_create(&k2, NULL);
    check("the deleted key is reused", k2 == key, (long long)k2);
    check("a reused key reads NULL in every thread (main's old value is gone)", pthread_getspecific(k2) == NULL, 0);
    pthread_key_delete(k2);
    return finish();
}
