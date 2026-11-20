/* poly-mac: libsystem
 * semaphore_destroy(task, sem) destroys the semaphore and takes the caller's send right away: the name is invalid afterwards (a wait or signal on it
 * is KERN_INVALID_NAME (15)); a second destroy of the name is MACH_SEND_INVALID_RIGHT (0x1000000a); a thread blocked in semaphore_wait when
 * the semaphore is destroyed returns KERN_TERMINATED (37). */
#include "poly_mexc.h"
#include <mach/semaphore.h>
#include <mach/sync_policy.h>
static semaphore_t sem;
static volatile kern_return_t result;
static void *waiter(void *a) {
    (void)a;
    result = semaphore_wait(sem);
    return NULL;
}
int test_main(void) {
    mach_port_t t = mach_task_self();
    semaphore_t s = 0;
    semaphore_create(t, &s, SYNC_POLICY_FIFO, 0);
    check("destroy succeeds", semaphore_destroy(t, s) == KERN_SUCCESS, 0);
    check("the name is invalid afterwards", exc_type_of(s) == (0xdead0000u | KERN_INVALID_NAME), exc_type_of(s));
    check("a wait on it: KERN_INVALID_NAME", semaphore_wait(s) == KERN_INVALID_NAME, 0);
    check("a signal on it: KERN_INVALID_NAME", semaphore_signal(s) == KERN_INVALID_NAME, 0);
    kern_return_t kr = semaphore_destroy(t, s);
    check("a second destroy: MACH_SEND_INVALID_RIGHT", kr == MACH_SEND_INVALID_RIGHT, kr);
    semaphore_create(t, &sem, SYNC_POLICY_FIFO, 0);
    pthread_t w;
    result = -1;
    pthread_create(&w, NULL, waiter, NULL);
    usleep(300000);
    check("the waiter is still waiting", result == -1, result);
    check("destroy with a waiter succeeds", semaphore_destroy(t, sem) == KERN_SUCCESS, 0);
    pthread_join(w, NULL);
    check("and the waiter returned KERN_TERMINATED", result == KERN_TERMINATED, result);
    return finish();
}
