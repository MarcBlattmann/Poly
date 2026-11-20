/* pthread_key_create: destructors of several keys run for one ending
 * thread; a destructor that sets a value again is run again (up to
 * PTHREAD_DESTRUCTOR_ITERATIONS = 4). */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>

static pthread_key_t ka, kb, kc;
static int ran_a, ran_b, again_calls;

static void da(void *v) { ran_a = (int)(intptr_t)v; }
static void db(void *v) { ran_b = (int)(intptr_t)v; }
static void again(void *v) {
    again_calls++;
    pthread_setspecific(kc, v); /* arm it again, forever */
}

static void *worker(void *arg) {
    (void)arg;
    pthread_setspecific(ka, (void *)11);
    pthread_setspecific(kb, (void *)22);
    pthread_setspecific(kc, (void *)33);
    return NULL;
}

int test_main(void) {
    pthread_key_create(&ka, da);
    pthread_key_create(&kb, db);
    pthread_key_create(&kc, again);
    pthread_t t;
    pthread_create(&t, NULL, worker, NULL);
    pthread_join(t, NULL);
    check("destructor of key a ran with 11", ran_a == 11, ran_a);
    check("destructor of key b ran with 22", ran_b == 22, ran_b);
    check("a destructor that re-arms its key is called 4 times, then dropped", again_calls == 4, again_calls);
    value("PTHREAD_DESTRUCTOR_ITERATIONS", 4);
    return finish();
}
