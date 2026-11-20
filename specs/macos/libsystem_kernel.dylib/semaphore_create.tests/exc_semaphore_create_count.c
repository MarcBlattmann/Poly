/* poly-mac: libsystem
 * The initial value is the semaphore's count: with value 2 two waits return at once, a third with a timeout gives KERN_OPERATION_TIMED_OUT (49);
 * a signal then lets one more wait through; with value 0 a wait with a timeout times out; semaphore_signal_all with nobody waiting does not
 * raise the count. */
#include "poly_mexc.h"
#include <mach/semaphore.h>
#include <mach/sync_policy.h>
int test_main(void) {
    mach_port_t t = mach_task_self();
    semaphore_t s = 0;
    semaphore_create(t, &s, SYNC_POLICY_FIFO, 2);
    mach_timespec_t quick = {0, 50000000};
    check("first wait", semaphore_wait(s) == KERN_SUCCESS, 0);
    check("second wait", semaphore_wait(s) == KERN_SUCCESS, 0);
    check("third wait with a timeout: KERN_OPERATION_TIMED_OUT", semaphore_timedwait(s, quick) == KERN_OPERATION_TIMED_OUT, 0);
    check("a signal", semaphore_signal(s) == KERN_SUCCESS, 0);
    check("lets one wait through", semaphore_wait(s) == KERN_SUCCESS, 0);
    check("and then it is empty again", semaphore_timedwait(s, quick) == KERN_OPERATION_TIMED_OUT, 0);
    check("signal_all with nobody waiting succeeds", semaphore_signal_all(s) == KERN_SUCCESS, 0);
    check("and does not raise the count", semaphore_timedwait(s, quick) == KERN_OPERATION_TIMED_OUT, 0);
    semaphore_t z = 0;
    semaphore_create(t, &z, SYNC_POLICY_FIFO, 0);
    check("value 0: a wait with a timeout times out", semaphore_timedwait(z, quick) == KERN_OPERATION_TIMED_OUT, 0);
    return finish();
}
