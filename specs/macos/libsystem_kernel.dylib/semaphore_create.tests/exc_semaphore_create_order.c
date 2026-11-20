/* poly-mac: libsystem
 * The policy decides which waiter a signal releases: with SYNC_POLICY_FIFO the thread that waited first, with SYNC_POLICY_LIFO the one that waited last.
 * Two threads wait one after the other (a pause between, so that the order is known) and one signal is given. */
#include "poly_mexc.h"
#include <mach/semaphore.h>
#include <mach/sync_policy.h>
static semaphore_t sem;
static volatile int order[2], woken;
static void *waiter(void *a) {
    int me = (int)(long)a;
    semaphore_wait(sem);
    order[woken++] = me;
    return NULL;
}
static int first_woken(int policy) {
    mach_port_t t = mach_task_self();
    semaphore_create(t, &sem, policy, 0);
    woken = 0;
    order[0] = order[1] = 0;
    pthread_t a, b;
    pthread_create(&a, NULL, waiter, (void *)1L);
    usleep(300000);
    pthread_create(&b, NULL, waiter, (void *)2L);
    usleep(300000);
    semaphore_signal(sem);
    usleep(300000);
    int first = order[0];
    semaphore_signal(sem);
    pthread_join(a, NULL);
    pthread_join(b, NULL);
    return first;
}
int test_main(void) {
    check("SYNC_POLICY_FIFO: the first waiter is released first", first_woken(SYNC_POLICY_FIFO) == 1, 0);
    check("SYNC_POLICY_LIFO: the last waiter is released first", first_woken(SYNC_POLICY_LIFO) == 2, 0);
    return finish();
}
